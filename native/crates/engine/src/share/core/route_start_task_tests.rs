//! Start bridge over real QUIC (Z-R2): a private Iroh relay on loopback, the
//! production route monitor and payload gate, and "files" as one request
//! stream each (header, bytes, digest acknowledgement), like Share requests.
//! Linux only: the relay fixture needs Iroh's `test-utils` dev feature.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, QuicTransportConfig};
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, TransportAddr};
use sha2::{Digest, Sha256};

use super::super::super::route_monitor;
use super::super::super::route_protocol;
use super::super::super::route_state::{RouteClass, RoutePhase};

const ALPN: &[u8] = b"smart-explorer/route-start-task/1";
/// Route monitor label of the client side (diagnostics filter).
pub(super) const CLIENT: &str = "route-start-client";
const HOST: &str = "route-start-host";
pub(super) const FILE: usize = 256 * 1024;
pub(super) const WAIT: Duration = Duration::from_secs(30);

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("test runtime")
}

async fn endpoint(map: &RelayMap, direct: bool) -> Endpoint {
    let builder = Endpoint::builder(iroh::endpoint::presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .clear_address_lookup()
        .relay_mode(RelayMode::Custom(map.clone()))
        // The private test relay uses a new self-signed certificate.
        .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
        .transport_config(
            QuicTransportConfig::builder()
                .application_path_policy(true)
                .build(),
        );
    let builder = if direct {
        builder
    } else {
        builder.clear_ip_transports()
    };
    builder.bind().await.expect("private relay endpoint")
}

pub(super) struct Pair {
    pub(super) client: Connection,
    pub(super) host: Connection,
    /// Before `connect`: the origin of time to first byte.
    pub(super) started: Instant,
    pub(super) ledger: Arc<Ledger>,
    endpoints: [Endpoint; 2],
}

impl Pair {
    /// Both endpoints know only the relay address of each other: a LAN or
    /// direct path can appear only later through Iroh's NAT traversal.
    pub(super) async fn new(map: &RelayMap, url: &RelayUrl, direct: bool, monitor: bool) -> Self {
        let host_endpoint = endpoint(map, direct).await;
        let client_endpoint = endpoint(map, direct).await;
        tokio::time::timeout(WAIT, async {
            tokio::join!(host_endpoint.online(), client_endpoint.online());
        })
        .await
        .expect("both endpoints register at the private relay");
        let address =
            EndpointAddr::from_parts(host_endpoint.id(), [TransportAddr::Relay(url.clone())]);
        let started = Instant::now();
        let (client, host) = tokio::time::timeout(WAIT, async {
            tokio::join!(client_endpoint.connect(address, ALPN), async {
                host_endpoint.accept().await.expect("accept").await
            })
        })
        .await
        .expect("relay handshake deadline");
        let (client, host) = (client.expect("client"), host.expect("host"));
        if monitor {
            // As `await_route` does on both sides of a Share session.
            route_monitor::start(&client, CLIENT.into(), false, true, Vec::new())
                .expect("client route monitor");
            route_monitor::start(&host, HOST.into(), false, true, Vec::new())
                .expect("host route monitor");
        }
        let ledger = Arc::new(Ledger::default());
        tokio::spawn(serve(host.clone(), ledger.clone()));
        Self {
            client,
            host,
            started,
            ledger,
            endpoints: [client_endpoint, host_endpoint],
        }
    }

    pub(super) async fn close(self) {
        self.client.close(0u32.into(), b"route start task done");
        self.host.close(0u32.into(), b"route start task done");
        let [client, host] = self.endpoints;
        tokio::join!(client.close(), host.close());
    }
}

/// What the host received: every file id with its digest and count.
#[derive(Default)]
pub(super) struct Ledger {
    pub(super) files: Mutex<HashMap<u64, ([u8; 32], u32)>>,
    first_byte: Mutex<Option<Instant>>,
}

impl Ledger {
    pub(super) fn first_byte(&self) -> Option<Instant> {
        *self.first_byte.lock().unwrap()
    }

    /// Every id in `0..sent` exactly once with the digest of its content.
    pub(super) fn assert_exactly_once(&self, sent: u64, len: usize) {
        let files = self.files.lock().unwrap();
        assert_eq!(files.len() as u64, sent, "no file lost or extra");
        for id in 0..sent {
            let (digest, count) = files.get(&id).unwrap_or_else(|| panic!("file {id} lost"));
            assert_eq!(*count, 1, "file {id} arrived {count} times");
            assert_eq!(*digest, sha(&content(id, len)), "file {id} corrupted");
        }
    }
}

pub(super) fn content(id: u64, len: usize) -> Vec<u8> {
    (0..len as u64)
        .map(|index| (index.wrapping_mul(2_654_435_761) ^ id.wrapping_mul(97)) as u8)
        .collect()
}

pub(super) fn sha(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

async fn serve(connection: Connection, ledger: Arc<Ledger>) {
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let ledger = ledger.clone();
        tokio::spawn(async move {
            let mut header = [0u8; 16];
            if recv.read_exact(&mut header).await.is_err() {
                return;
            }
            if let Ok(mut first) = ledger.first_byte.lock() {
                first.get_or_insert_with(Instant::now);
            }
            let id = u64::from_be_bytes(header[..8].try_into().unwrap());
            let len = u64::from_be_bytes(header[8..].try_into().unwrap()) as usize;
            if len > 64 * FILE {
                return;
            }
            let mut data = vec![0u8; len];
            if recv.read_exact(&mut data).await.is_err() {
                return;
            }
            let digest = sha(&data);
            ledger
                .files
                .lock()
                .unwrap()
                .entry(id)
                .and_modify(|entry| entry.1 += 1)
                .or_insert((digest, 1));
            let mut ack = [0u8; 40];
            ack[..8].copy_from_slice(&id.to_be_bytes());
            ack[8..].copy_from_slice(&digest);
            if send.write_all(&ack).await.is_ok() {
                let _ = send.finish();
            }
        });
    }
}

/// One file as one request stream; succeeds only on the host's matching digest.
pub(super) async fn send_file(connection: &Connection, id: u64, len: usize) -> io::Result<()> {
    let data = content(id, len);
    let (mut send, mut recv) = connection.open_bi().await.map_err(io::Error::other)?;
    let mut header = [0u8; 16];
    header[..8].copy_from_slice(&id.to_be_bytes());
    header[8..].copy_from_slice(&(len as u64).to_be_bytes());
    send.write_all(&header).await.map_err(io::Error::other)?;
    send.write_all(&data).await.map_err(io::Error::other)?;
    send.finish().map_err(io::Error::other)?;
    let mut ack = [0u8; 40];
    recv.read_exact(&mut ack).await.map_err(io::Error::other)?;
    if ack[..8] != id.to_be_bytes() || ack[8..] != sha(&data) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "acknowledgement",
        ));
    }
    Ok(())
}

/// One file through the production admission (`run_with_route`): constructed
/// once, never restarted by a route change.
pub(super) async fn send_routed(connection: &Connection, id: u64, len: usize) -> io::Result<()> {
    route_protocol::run_with_route(connection, WAIT, None, send_file(connection, id, len)).await
}

/// Application payload this side sent over relay paths that still exist.
pub(super) fn relay_tx(connection: &Connection) -> u64 {
    let paths = connection.paths();
    connection
        .application_path_policy()
        .paths
        .iter()
        .filter(|observed| {
            paths
                .iter()
                .find(|path| path.id() == observed.token.path_id())
                .is_some_and(|path| path.is_relay())
        })
        .map(|observed| observed.application_tx_bytes)
        .fold(0, u64::saturating_add)
}

#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub(super) at: Duration,
    pub(super) phase: RoutePhase,
    pub(super) class: Option<RouteClass>,
    pub(super) pause_epoch: u64,
    pub(super) relay_tx: u64,
}

/// Every route status change of `connection` with this side's relay payload.
pub(super) fn trace(connection: &Connection, origin: Instant) -> Arc<Mutex<Vec<Seen>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut status = route_monitor::subscribe(connection).expect("route monitor");
    let connection = connection.clone();
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let current = status.borrow_and_update().clone();
            record.lock().unwrap().push(Seen {
                at: origin.elapsed(),
                phase: current.phase,
                class: current.class,
                pause_epoch: current.pause_epoch,
                relay_tx: relay_tx(&connection),
            });
            if status.changed().await.is_err() || connection.close_reason().is_some() {
                break;
            }
        }
    });
    seen
}

fn lan_or_direct(seen: &Seen) -> bool {
    matches!(seen.class, Some(RouteClass::Lan | RouteClass::Direct))
}

/// After the first LAN/direct admission no relay admission follows and this
/// side's relay payload never grows again. Returns the switch, if any.
pub(super) fn assert_no_relay_after_direct(seen: &[Seen], end_relay_tx: u64) -> Option<Seen> {
    let first = seen.iter().position(lan_or_direct)?;
    let switch = seen[first].clone();
    for later in &seen[first..] {
        assert_ne!(later.class, Some(RouteClass::Relay), "{seen:?}");
        assert!(later.relay_tx <= switch.relay_tx, "{seen:?}");
    }
    assert!(end_relay_tx <= switch.relay_tx, "{seen:?}");
    Some(switch)
}

/// Sends files over `lanes` parallel request lanes until `stop` says so.
/// Returns how many files were sent (ids `0..n`, all acknowledged).
pub(super) async fn drive(
    connection: &Connection,
    lanes: usize,
    stop: impl Fn(u64) -> bool + Send + Sync + 'static,
) -> u64 {
    let next = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicU64::new(0));
    let halt = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(stop);
    let mut tasks = Vec::new();
    for _ in 0..lanes {
        let (connection, next, done, halt, stop) = (
            connection.clone(),
            next.clone(),
            done.clone(),
            halt.clone(),
            stop.clone(),
        );
        tasks.push(tokio::spawn(async move {
            while !halt.load(Ordering::Acquire) {
                let id = next.fetch_add(1, Ordering::AcqRel);
                let sent = send_routed(&connection, id, FILE).await;
                sent.expect("file over the route");
                if stop(done.fetch_add(1, Ordering::AcqRel) + 1) {
                    halt.store(true, Ordering::Release);
                }
            }
        }));
    }
    for task in tasks {
        task.await.expect("lane");
    }
    next.load(Ordering::Acquire)
}

fn is_direct(connection: &Connection) -> bool {
    route_monitor::snapshot(connection)
        .is_some_and(|status| matches!(status.class, Some(RouteClass::Lan | RouteClass::Direct)))
}

#[test]
fn route_start_task_relay_only_peer_starts_at_once_and_keeps_working() {
    runtime().block_on(async {
        let relay = iroh::test_utils::run_relay_server_with(false).await;
        let (map, url, _relay) = relay.expect("private relay");
        let pair = Pair::new(&map, &url, false, true).await;
        let seen = trace(&pair.client, pair.started);
        send_routed(&pair.client, 0, FILE)
            .await
            .expect("first file");
        let at_first = route_monitor::snapshot(&pair.client).expect("status");
        // The first file was carried while the direct phase was still running.
        assert_eq!(at_first.phase, RoutePhase::RelayStart, "{at_first:?}");
        assert!(at_first.application_allowed);
        let mut status = route_monitor::subscribe(&pair.client).unwrap();
        tokio::time::timeout(Duration::from_secs(90), async {
            while status.borrow_and_update().phase != RoutePhase::RelayFallback {
                status.changed().await.expect("monitor");
            }
        })
        .await
        .expect("finite direct phase ends in the marked fallback");
        let fallback = status.borrow().clone();
        assert!(fallback.application_allowed);
        assert_eq!(fallback.class, Some(RouteClass::Relay));
        assert_eq!(fallback.pause_epoch, at_first.pause_epoch, "no pause");
        for id in 1..9 {
            let sent = send_routed(&pair.client, id, FILE).await;
            sent.expect("file after fallback");
        }
        pair.ledger.assert_exactly_once(9, FILE);
        let seen = seen.lock().unwrap().clone();
        assert!(!seen.iter().any(lan_or_direct), "{seen:?}");
        pair.close().await;
    });
}

#[test]
fn route_start_task_monitor_switches_to_direct_without_duplicate_or_lost_files() {
    runtime().block_on(async {
        let relay = iroh::test_utils::run_relay_server_with(false).await;
        let (map, url, _relay) = relay.expect("private relay");
        let pair = Pair::new(&map, &url, true, true).await;
        let client_seen = trace(&pair.client, pair.started);
        let host_seen = trace(&pair.host, pair.started);
        let watched = pair.client.clone();
        let after_switch = Arc::new(AtomicU64::new(u64::MAX));
        let marker = after_switch.clone();
        let deadline = Instant::now() + Duration::from_secs(60);
        // Keep sending until 16 files completed after the switch to direct.
        let sent = drive(&pair.client, 4, move |done| {
            if is_direct(&watched) {
                let first = u64::MAX;
                let _ = marker.compare_exchange(first, done, Ordering::AcqRel, Ordering::Acquire);
            }
            let switched = marker.load(Ordering::Acquire);
            (switched != u64::MAX && done >= switched + 16) || Instant::now() >= deadline
        })
        .await;
        let switched_at = after_switch.load(Ordering::Acquire);
        assert_ne!(switched_at, u64::MAX, "no LAN/direct path in 60 s");
        pair.ledger.assert_exactly_once(sent, FILE);
        let client_seen = client_seen.lock().unwrap().clone();
        let host_seen = host_seen.lock().unwrap().clone();
        let switch = assert_no_relay_after_direct(&client_seen, relay_tx(&pair.client));
        assert_no_relay_after_direct(&host_seen, relay_tx(&pair.host));
        let relay_first = client_seen
            .iter()
            .any(|seen| seen.phase == RoutePhase::RelayStart);
        println!(
            "route_start_task switch: files={sent} relay_first={relay_first} switch={switch:?} client={client_seen:?}"
        );
        pair.close().await;
    });
}
