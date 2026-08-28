//! Smart Explorer share signaling and Iroh relay server.
//!
//! The server is intentionally untrusted: it stores and routes signed presence
//! blobs for persistent direct contacts and rooms and, by default, forwards
//! end-to-end encrypted Iroh transport traffic on the adjacent port. It cannot
//! decrypt relation secrets, private keys, file names, file contents, or export
//! configuration. Clients validate HMAC proofs, pinned SmartExplorer identities,
//! and Iroh NodeIds before opening a peer session. Public discovery exposes only
//! short-lived aliases and relays opaque PAKE/application packets without seeing
//! PINs, stable relation identifiers, or decrypted key bundles.

use std::net::{Shutdown, TcpListener};
use std::sync::{Arc, Mutex};

mod direct_messages;
mod direct_validation;
mod discovery;
mod discovery_state;
mod limits;
mod line;
#[cfg(test)]
mod main_tests;
#[cfg(test)]
mod mixed_version_tests;
mod protocol;
mod rate_limits;
mod registration_guard;
mod relay;
#[cfg(test)]
mod resource_limits_tests;
#[cfg(test)]
mod share_remote_task_tests;
#[cfg(test)]
mod share_remote_wire_task_tests;
mod state;
#[cfg(test)]
mod state_transition_tests;
mod tracked_direct;
#[cfg(test)]
mod tracked_direct_tests;
mod transport;
#[cfg(test)]
mod transport_cleanup_tests;
mod websocket_read_limit;
mod writer;
use limits::{
    ConnectionLimiter, SourceClassifier, MAX_CONNECTIONS_PER_SOURCE, MAX_CONNECTION_WORKERS,
};
use protocol::{In, Out, PeerPresence};
use rate_limits::AcceptRateLimiter;
use state::{join_room, leave_room, State};
use transport::handle_with_source;
use writer::Writer;

fn send(writer: &Writer, message: &Out) -> bool {
    writer.try_send(message)
}

fn main() {
    let bind = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SE_SHARE_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:51820".to_string());
    let source_classifier = match trusted_proxy_sources_from_env() {
        Ok(classifier) => classifier,
        Err(error) => {
            eprintln!("se-share-server: {error}");
            std::process::exit(1);
        }
    };
    let _relay_guard = match relay::start(&bind) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("se-share-server: {error}");
            std::process::exit(1);
        }
    };
    let listener = match TcpListener::bind(&bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("se-share-server: cannot bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("se-share-server signaling on {bind} (raw TCP + WebSocket upgrade)");
    let state = Arc::new(Mutex::new(State::default()));
    let connections = ConnectionLimiter::new(MAX_CONNECTION_WORKERS, MAX_CONNECTIONS_PER_SOURCE);
    let mut accept_rate = AcceptRateLimiter::new();
    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let source = match stream.peer_addr() {
            Ok(address) => source_classifier.classify(address),
            Err(_) => {
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
        };
        let Some(permit) = connections.try_acquire(source) else {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        };
        if !accept_rate.try_admit(source) {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        let state = state.clone();
        let _ = std::thread::Builder::new()
            .name("share-server-connection".into())
            .spawn(move || {
                let _permit = permit;
                let _ = handle_with_source(stream, state, source);
            });
    }
}

fn trusted_proxy_sources_from_env() -> Result<SourceClassifier, String> {
    match std::env::var("SE_SHARE_TRUSTED_PROXY_IPS") {
        Ok(value) => SourceClassifier::parse_proxy_ips(&value),
        Err(std::env::VarError::NotPresent) => Ok(SourceClassifier::default()),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("SE_SHARE_TRUSTED_PROXY_IPS is not valid Unicode".into())
        }
    }
}

fn dispatch(id: u64, writer: &Writer, msg: In, state: &Arc<Mutex<State>>) {
    match msg {
        In::PublishDirect { presence } => tracked_direct::publish(id, writer, presence, state),
        In::UnpublishDirect { lookup_id } => tracked_direct::unpublish(id, &lookup_id, state),
        In::WatchDirect { lookup_id } => tracked_direct::watch(id, writer, &lookup_id, state),
        In::RequestDirect {
            lookup_id,
            presence,
        } => tracked_direct::request_legacy(writer, &lookup_id, presence, state),
        In::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg,
        } => tracked_direct::decision_legacy(
            writer,
            &lookup_id,
            &requester_device_id,
            accepted,
            presence,
            msg,
            state,
        ),
        In::SubmitDirectRequest {
            request,
            legacy_presence,
        } => tracked_direct::route_request(id, writer, *request, legacy_presence, state),
        In::SubmitDirectRequestReceipt { receipt } => {
            tracked_direct::route_request_receipt(id, writer, receipt, state)
        }
        In::SubmitDirectDecision { decision } => {
            tracked_direct::route_decision(id, writer, decision, state)
        }
        In::SubmitDirectDecisionReceipt { receipt } => {
            tracked_direct::route_decision_receipt(id, writer, receipt, state)
        }
        In::UnwatchDirect { lookup_id } => tracked_direct::unwatch(id, &lookup_id, state),
        In::JoinRoom { room_id, presence } => join_room(id, writer, &room_id, presence, state),
        In::LeaveRoom { room_id } => leave_room(id, &room_id, state),
        In::PublishDiscovery { offer } => discovery::publish(id, writer, offer, state),
        In::UnpublishDiscovery { offer_id } => discovery::unpublish(id, writer, &offer_id, state),
        In::ListDiscoveries => discovery::list(id, writer, state),
        In::StartPairing {
            discovery_id,
            exchange_id,
            payload,
        } => discovery::start_pairing(id, writer, &discovery_id, &exchange_id, payload, state),
        In::PairingPacket {
            exchange_id,
            kind,
            payload,
        } => discovery::pairing_packet(id, writer, &exchange_id, kind, payload, state),
        In::CancelPairing { exchange_id } => {
            discovery::cancel_pairing(id, writer, &exchange_id, state)
        }
        In::Heartbeat => {
            discovery::prune_expired(state);
            send(writer, &Out::Pong);
        }
        In::Hello { .. } => {}
    }
}
