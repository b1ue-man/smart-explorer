//! P2b of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (E4): the host
//! gives its download streams priority -1 and keeps every other reply at 0.
//! Remote suite only (AGENTS.md).
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr};

use super::*;
use crate::share::keepalive::transport_purpose::overrides::{hold, ProfileOverrides};
use crate::share::keepalive::transport_purpose::TRANSFER_STREAM_PRIORITY;

fn read() -> FsRequest {
    FsRequest::Read {
        path: "a.bin".into(),
        lz4: false,
    }
}

fn read_at() -> FsRequest {
    FsRequest::ReadAt {
        path: "a.bin".into(),
        id: None,
        offset: 0,
        len: 0,
        lz4: false,
        evidence: false,
    }
}

fn get_batch() -> FsRequest {
    FsRequest::GetBatch {
        items: Vec::new(),
        evidence: false,
    }
}

/// Requests whose replies are navigation, listings, status or a few bytes of
/// outcome after the client's own data.
fn light_requests() -> Vec<FsRequest> {
    let path = || "dir".to_string();
    vec![
        FsRequest::ListDir { path: path() },
        FsRequest::Stat { path: path() },
        FsRequest::WalkTree { path: path() },
        FsRequest::StorageSnapshot { path: path() },
        FsRequest::Capacity { path: path() },
        FsRequest::MkdirAll { path: path() },
        FsRequest::RemoveFile { path: path() },
        FsRequest::Write {
            path: path(),
            lz4: false,
        },
        FsRequest::PutBatch {
            nonce: "n".into(),
            entries: Vec::new(),
            evidence: false,
        },
        FsRequest::ReleaseLease,
    ]
}

#[test]
fn verbindungen_task_host_download_streams_are_exactly_the_data_reads() {
    for request in [read(), read_at(), get_batch()] {
        assert!(streams_file_data(&request), "{request:?}");
    }
    for request in light_requests() {
        assert!(!streams_file_data(&request), "{request:?}");
    }
}

async fn loopback() -> Endpoint {
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .expect("loopback bind address")
        .alpns(vec![crate::share::node::ALPN.to_vec()])
        .bind()
        .await
        .expect("bind loopback endpoint")
}

/// A connected loopback pair: the client side of the file session and the
/// host's accepted connection. The endpoints use the plain transport
/// configuration, so data of a test stream never waits for a path decision.
async fn pair(client: &Endpoint, server: &Endpoint) -> Option<(Connection, Connection)> {
    let address = EndpointAddr::from_parts(
        server.id(),
        server
            .bound_sockets()
            .into_iter()
            .map(iroh::TransportAddr::Ip),
    );
    let (outgoing, incoming) =
        tokio::join!(client.connect(address, crate::share::node::ALPN), async {
            server.accept().await?.await.ok()
        });
    Some((outgoing.ok()?, incoming?))
}

/// The priority the host's send half has after `prioritize_download`, for
/// each request, on streams of one real connection.
async fn host_priorities(requests: Vec<FsRequest>) -> Option<Vec<(bool, i32)>> {
    let client = loopback().await;
    let server = loopback().await;
    let exchange = async {
        let (outgoing, incoming) = pair(&client, &server).await?;
        let mut seen = Vec::new();
        for request in requests {
            // A stream becomes visible to the host with its first bytes.
            let (mut send, _recv) = outgoing.open_bi().await.ok()?;
            send.write_all(b"x").await.ok()?;
            let (host_send, _host_recv) = incoming.accept_bi().await.ok()?;
            assert_eq!(host_send.priority().ok(), Some(0), "noq default");
            let download = prioritize_download(&host_send, &request);
            seen.push((download, host_send.priority().ok()?));
        }
        Some(seen)
    };
    let seen = tokio::time::timeout(Duration::from_secs(10), exchange)
        .await
        .ok()
        .flatten();
    client.close().await;
    server.close().await;
    seen
}

#[tokio::test]
async fn verbindungen_task_host_download_streams_get_priority_minus_one_replies_keep_zero() {
    let _plan = hold(ProfileOverrides::default());
    let mut requests = vec![read(), read_at(), get_batch()];
    let downloads = requests.len();
    requests.extend(light_requests());
    let total = requests.len();
    let seen = host_priorities(requests).await.expect("loopback exchange");
    assert_eq!(seen.len(), total);
    for (download, priority) in &seen[..downloads] {
        assert!(*download);
        assert_eq!(*priority, TRANSFER_STREAM_PRIORITY);
        assert_eq!(*priority, -1);
    }
    for (download, priority) in &seen[downloads..] {
        assert!(!*download);
        assert_eq!(*priority, 0, "navigation, listing and status stay at 0");
    }
}

#[tokio::test]
async fn verbindungen_task_host_download_priority_follows_the_m3_switch() {
    // Measurement "vorher" of M3: with the switch off the host's download
    // streams keep 0 like the client's transfer streams.
    let _m3 = hold(ProfileOverrides {
        transfer_priority: Some(false),
        ..ProfileOverrides::default()
    });
    let seen = host_priorities(vec![read_at()])
        .await
        .expect("loopback exchange");
    assert_eq!(seen, vec![(true, 0)]);
}
