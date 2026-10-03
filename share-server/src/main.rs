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

mod access;
mod bindings;
mod config;
mod direct_messages;
mod direct_presence;
mod direct_validation;
mod discovery;
mod discovery_state;
mod hello_session;
mod idle;
mod idle_outbox;
#[cfg(test)]
mod idle_outbox_tests;
#[cfg(test)]
mod idle_transport_tests;
mod limits;
mod line;
mod login;
#[cfg(test)]
mod main_tests;
#[cfg(test)]
mod mixed_version_tests;
mod protocol;
mod rate_limits;
mod registration_guard;
mod relay;
mod relay_access;
#[cfg(test)]
mod resource_limits_tests;
mod rooms;
mod server_tls;
#[cfg(test)]
mod share_remote_task_tests;
#[cfg(test)]
mod share_remote_wire_task_tests;
#[cfg(test)]
mod signal_security_state_tests;
#[cfg(test)]
mod signal_security_transport_tests;
mod signal_session;
mod signal_stream;
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
mod websocket_socket;
mod writer;
use idle::SignalTiming;
use limits::{ConnectionLimiter, SourceClassifier};
use protocol::{In, Out, PeerPresence};
use rate_limits::AcceptRateLimiter;
use state::{leave_room, State};
use transport::handle_with_security;
use writer::Writer;

fn send(writer: &Writer, message: &Out) -> bool {
    writer.try_send(message)
}

fn main() {
    let options = match config::Options::from_env_and_args() {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{}", config::HELP);
            return;
        }
        Err(error) => fatal(&error),
    };
    let bind = options.bind.to_string();
    let relay_address =
        relay::bind_address(&bind).unwrap_or_else(|error| fatal(&error.to_string()));
    options
        .check_security(relay_address)
        .unwrap_or_else(|error| fatal(&error));
    let tls = match (&options.cert, &options.key) {
        (Some(cert), Some(key)) => {
            Some(server_tls::load(cert, key).unwrap_or_else(|error| fatal(&error)))
        }
        _ => None,
    };
    let mut initial_state = State::default();
    initial_state.policy = state::ServerPolicy {
        limits: options.limits,
        require_key_login: options.require_key_login,
    };
    if let Some(path) = &options.state_file {
        initial_state.bindings = bindings::Bindings::load(path, discovery_state::unix_seconds())
            .unwrap_or_else(|error| fatal(&error));
        initial_state.bindings.save(path).unwrap_or_else(|error| {
            fatal(&format!(
                "cannot write server state {}: {error}",
                path.display()
            ))
        });
        initial_state.binding_path = Some(path.clone());
    } else {
        eprintln!("se-share-server: device/lookup key bindings are memory-only; use --state-file for restart protection");
    }
    let relay_admissions = initial_state.relay_admissions.clone();
    let state = Arc::new(Mutex::new(initial_state));
    let source_classifier = match trusted_proxy_sources_from_env() {
        Ok(classifier) => classifier,
        Err(error) => {
            eprintln!("se-share-server: {error}");
            std::process::exit(1);
        }
    };
    let keepalive = match idle::keepalive_from_env() {
        Ok((keepalive, notice)) => {
            if let Some(notice) = notice {
                eprintln!("se-share-server: {notice}");
            }
            keepalive
        }
        Err(error) => {
            eprintln!("se-share-server: {error}");
            std::process::exit(1);
        }
    };
    let _relay_guard = match relay::start(
        relay_address,
        relay::RelayOptions {
            keepalive,
            tls: tls.as_ref().map(|config| config.as_ref().clone()),
            plaintext_fallback: options.allow_plaintext
                || relay_address.is_some_and(|addr| addr.ip().is_loopback()),
            trusted_proxy: relay_address
                .is_some_and(|addr| source_classifier.is_trusted_ip(addr.ip())),
            admissions: relay_admissions,
        },
    ) {
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
    eprintln!(
        "se-share-server signaling on {bind} ({}, idle keepalive {} s)",
        if tls.is_some() {
            "TLS WebSocket"
        } else {
            "explicit plaintext TCP/WebSocket"
        },
        keepalive.secs()
    );
    let timing = SignalTiming::new(keepalive);
    let connections = ConnectionLimiter::from_limits(&options.limits);
    let allow_plaintext = options.allow_plaintext || options.bind.ip().is_loopback();
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
        let timing = timing.clone();
        let tls = tls.clone();
        let _ = std::thread::Builder::new()
            .name("share-server-connection".into())
            .spawn(move || {
                // Idle connections keep this permit like any other connection.
                let _permit = permit;
                let _ = handle_with_security(stream, state, source, &timing, tls, allow_plaintext);
            });
    }
}

fn fatal(error: &str) -> ! {
    eprintln!("se-share-server: {error}");
    std::process::exit(1);
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
        In::PublishDirect {
            presence,
            access_hash,
        } => direct_presence::publish(id, writer, presence, access_hash, state),
        In::UnpublishDirect { lookup_id } => tracked_direct::unpublish(id, &lookup_id, state),
        In::WatchDirect {
            lookup_id,
            access_proof,
        } => direct_presence::watch(id, writer, &lookup_id, access_proof, state),
        In::RequestDirect {
            lookup_id,
            presence,
        } => {
            if direct_presence::require_origin(state, id, writer, &presence) {
                tracked_direct::request_legacy(writer, &lookup_id, presence, state);
            }
        }
        In::DirectAccessAccepted {
            lookup_id,
            requester_device_id,
            accepted,
            presence,
            msg,
        } => {
            if !direct_presence::require_owner(state, id, writer, &lookup_id)
                || presence.as_ref().is_some_and(|presence| {
                    !direct_presence::require_origin(state, id, writer, presence)
                })
            {
                return;
            }
            tracked_direct::decision_legacy(
                writer,
                &lookup_id,
                &requester_device_id,
                accepted,
                presence,
                msg,
                state,
            );
        }
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
        In::UnwatchDirect { lookup_id } => {
            tracked_direct::unwatch(id, &lookup_id, state);
            writer.forget_idle_direct(&lookup_id);
        }
        In::JoinRoom {
            room_id,
            presence,
            access_proof,
        } => rooms::join_with_access(id, writer, &room_id, presence, access_proof, state),
        In::LeaveRoom { room_id } => {
            leave_room(id, &room_id, state);
            writer.forget_idle_room(&room_id);
        }
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
        // Ignored unless `idle_keepalive_v1` was negotiated.
        In::SetIdle {
            idle,
            keepalive_secs,
        } => writer.set_idle(idle, keepalive_secs),
        // Any inbound line already renewed the connection's liveness.
        In::KeepaliveAck | In::Hello { .. } | In::HelloAuth { .. } => {}
    }
}
