//! P2 of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (E1, E4, E5):
//! purpose profiles, ALPN mapping, late-bound purpose and the controller a
//! profile builds. Remote suite only (AGENTS.md).
use std::time::{Duration, Instant};

use super::overrides::{hold, ProfileOverrides};
use super::*;
use crate::share::keepalive::yield_controller::YieldingController;

fn debug(config: &QuicTransportConfig) -> String {
    format!("{config:?}")
}

/// The endpoint configuration (`node.rs`) as text.
fn endpoint_text() -> String {
    debug(&crate::share::keepalive::iroh_transport_config_for_routes(
        true,
    ))
}

#[test]
fn verbindungen_task_profiles_keep_endpoint_values() {
    let _plan = hold(ProfileOverrides::default());
    let endpoint = endpoint_text();
    // The endpoint values every profile must keep (C9).
    for expected in [
        "application_path_policy: true",
        "max_concurrent_bidi_streams: 64",
        "max_concurrent_uni_streams: 0",
        "keep_alive_interval: Some(5s)",
        "default_path_keep_alive_interval: Some(5s)",
        "max_transmit_segments: 10",
        "max_transmit_datagrams: 20",
        "ack_frequency_config: None",
    ] {
        assert!(endpoint.contains(expected), "{expected} in {endpoint}");
    }
    for purpose in TransportPurpose::ALL {
        let profile = debug(&transport_config(purpose));
        // Apart from the send batches (and the controller, which has no
        // text), a profile is the endpoint configuration.
        let normalized = profile
            .replace("max_transmit_segments: 40", "max_transmit_segments: 10")
            .replace("max_transmit_datagrams: 80", "max_transmit_datagrams: 20");
        assert_eq!(normalized, endpoint, "{purpose:?}");
        let bulk = profile.contains("max_transmit_segments: 40")
            && profile.contains("max_transmit_datagrams: 80");
        assert_eq!(bulk, purpose.throughput(), "{purpose:?}: {profile}");
    }
    assert!(TransportPurpose::Files.throughput() && TransportPurpose::Tcp.throughput());
    for purpose in [
        TransportPurpose::Screen,
        TransportPurpose::Exec,
        TransportPurpose::Apps,
        TransportPurpose::LanLink,
    ] {
        assert!(!purpose.throughput() && !purpose.yields(), "{purpose:?}");
    }
}

#[test]
fn verbindungen_task_profile_switches_change_only_their_values() {
    let small = (count(10), count(20));
    {
        let _m1 = hold(ProfileOverrides {
            transmit: Some(small),
            ack_frequency: Some(true),
            ..ProfileOverrides::default()
        });
        let files = debug(&transport_config(TransportPurpose::Files));
        assert!(files.contains("max_transmit_segments: 10"), "{files}");
        assert!(files.contains("ack_frequency_config: Some("), "{files}");
        // Only bulk purposes take the switches.
        assert_eq!(
            debug(&transport_config(TransportPurpose::Screen)),
            endpoint_text()
        );
        assert_eq!(transfer_stream_priority(), TRANSFER_STREAM_PRIORITY);
    }
    {
        let _m3 = hold(ProfileOverrides {
            transfer_priority: Some(false),
            ..ProfileOverrides::default()
        });
        assert_eq!(transfer_stream_priority(), 0);
    }
    let _plan = hold(ProfileOverrides::default());
    assert_eq!(transfer_stream_priority(), -1);
    assert!(debug(&transport_config(TransportPurpose::Tcp)).contains("max_transmit_segments: 40"));
}

#[test]
fn verbindungen_task_profile_per_alpn() {
    use crate::share::{apps_wire, exec_protocol, lan_link_wire, tcp_forward};
    let cases: [(&[u8], TransportPurpose); 7] = [
        (crate::share::node::ALPN, TransportPurpose::Files),
        (exec_protocol::EXEC_ALPN, TransportPurpose::Exec),
        (tcp_forward::TCP_ALPN, TransportPurpose::Tcp),
        (tcp_forward::TCP_ALPN_V1, TransportPurpose::Tcp),
        (apps_wire::APPS_ALPN, TransportPurpose::Apps),
        (lan_link_wire::LAN_LINK_ALPN, TransportPurpose::LanLink),
        (
            crate::screen::core::wire::SCREEN_ALPN,
            TransportPurpose::Screen,
        ),
    ];
    for (alpn, purpose) in cases {
        assert_eq!(TransportPurpose::for_alpn(alpn), Some(purpose));
    }
    assert_eq!(
        TransportPurpose::for_alpn(b"smart-explorer/unknown/1"),
        None
    );
    assert_eq!(TransportPurpose::for_alpn(b""), None);
    // Every accepted ALPN has a profile.
    for alpn in accepted_alpns() {
        assert!(TransportPurpose::for_alpn(&alpn).is_some(), "{alpn:?}");
    }
    // The accepted list starts with the endpoint builder's list in
    // `node.rs`, so `accept_with` refuses nothing the endpoint accepted.
    let source = include_str!("node.rs");
    let start = source.find(".alpns(vec![").expect("ALPN list in node.rs");
    let block = &source[start + ".alpns(vec![".len()..];
    let block = &block[..block.find("])").expect("end of the ALPN list")];
    let names: Vec<&str> = block
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = entry.trim_end_matches(".to_vec()");
            entry.rsplit("::").next().unwrap_or(entry)
        })
        .collect();
    let value = |name: &str| -> Vec<u8> {
        match name {
            "ALPN" => crate::share::node::ALPN.to_vec(),
            "EXEC_ALPN" => exec_protocol::EXEC_ALPN.to_vec(),
            "TCP_ALPN" => tcp_forward::TCP_ALPN.to_vec(),
            "TCP_ALPN_V1" => tcp_forward::TCP_ALPN_V1.to_vec(),
            "APPS_ALPN" => apps_wire::APPS_ALPN.to_vec(),
            "LAN_LINK_ALPN" => lan_link_wire::LAN_LINK_ALPN.to_vec(),
            "SCREEN_ALPN" => crate::screen::core::wire::SCREEN_ALPN.to_vec(),
            other => panic!("ALPN {other} of node.rs has no profile"),
        }
    };
    let endpoint: Vec<Vec<u8>> = names.iter().map(|name| value(name)).collect();
    assert_eq!(endpoint.len(), 7, "{names:?}");
    assert_eq!(&accepted_alpns()[..endpoint.len()], &endpoint[..]);
}

#[test]
fn verbindungen_task_profile_late_bound_purpose() {
    let slot = PurposeSlot::new(None);
    assert_eq!(slot.purpose(), None);
    assert!(!slot.yields());
    slot.bind_alpn(b"smart-explorer/unknown/1");
    assert_eq!(slot.purpose(), None);
    slot.bind_alpn(crate::share::node::ALPN);
    assert_eq!(slot.purpose(), Some(TransportPurpose::Files));
    assert!(slot.yields());
    // A wrong hint is corrected by the negotiated ALPN.
    let hinted = PurposeSlot::new(Some(TransportPurpose::Files));
    hinted.bind_alpn(crate::share::exec_protocol::EXEC_ALPN);
    assert_eq!(hinted.purpose(), Some(TransportPurpose::Exec));
    assert!(!hinted.yields());
    // Offered ALPNs pick a profile only when they agree.
    use crate::share::tcp_forward::{TCP_ALPN, TCP_ALPN_V1};
    let ok = |list: &[&[u8]]| offered_purpose(list.iter().map(|alpn| Ok::<_, ()>(*alpn)));
    assert_eq!(ok(&[TCP_ALPN, TCP_ALPN_V1]), Some(TransportPurpose::Tcp));
    assert_eq!(
        ok(&[crate::share::node::ALPN]),
        Some(TransportPurpose::Files)
    );
    assert_eq!(ok(&[crate::share::node::ALPN, TCP_ALPN]), None);
    assert_eq!(ok(&[b"smart-explorer/unknown/1"]), None);
    assert_eq!(ok(&[]), None);
    assert_eq!(offered_purpose([Ok::<&[u8], ()>(TCP_ALPN), Err(())]), None);
}

fn built(factory: Arc<dyn ControllerFactory + Send + Sync>) -> Box<dyn std::any::Any> {
    factory.build(Instant::now(), 1200).into_any()
}

#[test]
fn verbindungen_task_profile_controller_per_purpose() {
    let _plan = hold(ProfileOverrides::default());
    for purpose in TransportPurpose::ALL {
        let slot = Arc::new(PurposeSlot::new(Some(purpose)));
        let controller = built(controller_factory(slot, Some(purpose)));
        assert_eq!(
            controller.is::<YieldingController>(),
            purpose.yields(),
            "{purpose:?}"
        );
        // A non-yielding purpose gets the environment's controller itself
        // (Cubic unless a measurement sets `SE_SHARE_QUIC_CC`, which a
        // keepalive test of this binary does for a moment).
        if !purpose.yields() {
            assert!(
                controller.is::<noq_proto::congestion::Cubic>()
                    || controller.is::<noq_proto::congestion::Bbr3>(),
                "{purpose:?}"
            );
        }
    }
    // Incoming connections always get the late-bound controller.
    let slot = Arc::new(PurposeSlot::new(None));
    assert!(built(controller_factory(slot, None)).is::<YieldingController>());
}

async fn loopback(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("loopback bind address")
        .alpns(alpns)
        .bind()
        .await
        .expect("bind loopback endpoint")
}

/// Connects with `alpn` and `purpose`; the server accepts through
/// [`AcceptProfiles`]. Returns the server's slot after binding and the
/// priority a client stream gets from [`prioritize_transfer`].
async fn accepted_purpose(
    alpn: &[u8],
    purpose: TransportPurpose,
) -> Option<(Option<TransportPurpose>, i32)> {
    let client = loopback(Vec::new()).await;
    let server = loopback(accepted_alpns()).await;
    let profiles = AcceptProfiles::new(&server);
    let address = EndpointAddr::from_parts(
        server.id(),
        server
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let exchange = async {
        let (outgoing, slot) = tokio::join!(connect(&client, address, alpn, purpose), async {
            let incoming = server.accept().await?;
            let (config, slot) = profiles.for_incoming(&incoming);
            // The hint is best effort: absent or right, never another purpose.
            assert!(slot.purpose().is_none_or(|hint| hint == purpose));
            let connection = incoming.accept_with(config).ok()?.await.ok()?;
            slot.bind_alpn(connection.alpn());
            Some((connection, slot))
        });
        let outgoing = outgoing.ok()?;
        let (_connection, slot) = slot?;
        // Opening a stream needs no path admission; only its data would.
        let (send, _recv) = outgoing.open_bi().await.ok()?;
        assert_eq!(send.priority().ok(), Some(0), "noq default");
        prioritize_transfer(&send);
        Some((slot.purpose(), send.priority().ok()?))
    };
    let bound = tokio::time::timeout(Duration::from_secs(10), exchange)
        .await
        .ok()
        .flatten();
    client.close().await;
    server.close().await;
    bound
}

#[tokio::test]
async fn verbindungen_task_profile_accept_with_binds_purpose_and_transfer_priority() {
    let _plan = hold(ProfileOverrides::default());
    assert_eq!(
        accepted_purpose(crate::share::node::ALPN, TransportPurpose::Files).await,
        Some((Some(TransportPurpose::Files), TRANSFER_STREAM_PRIORITY))
    );
    assert_eq!(
        accepted_purpose(
            crate::share::exec_protocol::EXEC_ALPN,
            TransportPurpose::Exec
        )
        .await,
        Some((Some(TransportPurpose::Exec), -1))
    );
}
