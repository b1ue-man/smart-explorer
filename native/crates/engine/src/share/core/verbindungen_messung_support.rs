//! Support of the measurements M1 to M4, M8 and M9 (plan 2026-11-14 C7, P7):
//! two endpoints on IPv4 loopback, connections with the profile of their
//! purpose (`transport_purpose.rs`) exactly as the share node makes them
//! (outgoing `connect`, incoming `accept_with` with a late-bound purpose),
//! each admitted by the route monitor like production connections. The shape
//! of the link comes from the suite (netem on `lo`, profile name in
//! `SE_SPEED_PROFILE`); this code only reports it. Only for `#[cfg(test)]`;
//! the measurements are in `verbindungen_messung_task_tests.rs`.
//!
//! Every measurement prints one line
//! `VERBINDUNGEN-MESSUNG fall=<M> plattform=<os-arch> variante=<name> profil=<link> <size>=<value> ...`
//! and fails only when it cannot measure, never on a threshold: the suite
//! evaluates the lines (C7).
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use tokio::task::JoinHandle;

use crate::share::keepalive::transport_purpose::{self, AcceptProfiles};
use crate::share::route_monitor;

// The module `keepalive` is private to `share`; the picture measurements of
// `screen` reach the purposes and the profile switches through this module.
pub(crate) use crate::share::keepalive::transport_purpose::overrides::{hold, ProfileOverrides};
pub(crate) use crate::share::keepalive::transport_purpose::TransportPurpose;

#[path = "verbindungen_messung_bulk.rs"]
pub(crate) mod bulk;
#[path = "verbindungen_messung_tunnel.rs"]
pub(crate) mod tunnel;

/// Upper bound of one measurement case (S12, E30: as for S4).
pub(crate) const CASE_BUDGET: Duration = Duration::from_secs(60);
/// Size of one write and one read.
const BLOCK_BYTES: usize = 64 * 1024;
/// Time the route monitor gets to admit a new connection.
const ADMISSION: Duration = Duration::from_secs(10);

/// `linux-x86_64`, `windows-x86_64`, ...
pub(crate) fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The link profile the suite set on `lo` (`lan`, `wan`, `loss`, ...);
/// `raw` without one.
pub(crate) fn link_profile() -> String {
    std::env::var("SE_SPEED_PROFILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "raw".to_owned())
}

/// One field of an output line.
pub(crate) fn field(name: &str, value: impl Display) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// The output line of one case and variant.
pub(crate) fn line(fall: &str, variant: &str, fields: Vec<(String, String)>) -> String {
    let mut text = format!(
        "VERBINDUNGEN-MESSUNG fall={fall} plattform={} variante={variant} profil={}",
        platform(),
        link_profile()
    );
    for (name, value) in fields {
        text.push_str(&format!(" {name}={value}"));
    }
    text
}

/// A measured value with one decimal.
pub(crate) fn one_decimal(value: f64) -> String {
    format!("{value:.1}")
}

/// Runs `future` on a runtime of its own within `CASE_BUDGET`.
pub(crate) fn run_case<T>(future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Laufzeit: {error}"))?;
    runtime.block_on(async {
        match tokio::time::timeout(CASE_BUDGET, future).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Messfall ueberschreitet {} s",
                CASE_BUDGET.as_secs()
            )),
        }
    })
}

fn text(error: impl Display) -> String {
    error.to_string()
}

/// Throughput in MiB/s.
pub(crate) fn mib_s(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64().max(f64::MIN_POSITIVE)
}

/// Median of three values.
pub(crate) fn median(mut values: [f64; 3]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[1]
}

/// Background tasks that end with their owner.
#[derive(Default)]
pub(crate) struct Tasks(Vec<JoinHandle<()>>);

impl Tasks {
    pub(crate) fn spawn(&mut self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.push(tokio::spawn(task));
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Two endpoints on loopback and the connections between them.
pub(crate) struct Link {
    client: Endpoint,
    server: Endpoint,
    profiles: AcceptProfiles,
    address: EndpointAddr,
}

async fn loopback(alpns: Vec<Vec<u8>>) -> Result<Endpoint, String> {
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .map_err(|error| format!("Bindadresse: {error:?}"))?
        .alpns(alpns)
        .bind()
        .await
        .map_err(|error| format!("Endpunkt: {error:?}"))
}

impl Link {
    pub(crate) async fn new() -> Result<Self, String> {
        let client = loopback(Vec::new()).await?;
        let server = loopback(transport_purpose::accepted_alpns()).await?;
        let profiles = AcceptProfiles::new(&server);
        let address = EndpointAddr::from_parts(
            server.id(),
            server.bound_sockets().into_iter().map(TransportAddr::Ip),
        );
        Ok(Self {
            client,
            server,
            profiles,
            address,
        })
    }

    /// One connection with `alpn` and the profile of `purpose`, admitted on
    /// both sides: (client side, server side).
    pub(crate) async fn connect(
        &self,
        alpn: &[u8],
        purpose: TransportPurpose,
    ) -> Result<(Connection, Connection), String> {
        let (outgoing, accepted) = tokio::join!(
            transport_purpose::connect(&self.client, self.address.clone(), alpn, purpose),
            self.accept_one()
        );
        let outgoing = outgoing.map_err(|error| format!("Verbindung: {error:?}"))?;
        let accepted = accepted?;
        route_monitor::start(
            &outgoing,
            self.client.id().to_string(),
            false,
            true,
            Vec::new(),
        )
        .map_err(|error| format!("Routemonitor (Client): {error}"))?;
        route_monitor::start(
            &accepted,
            self.server.id().to_string(),
            false,
            true,
            Vec::new(),
        )
        .map_err(|error| format!("Routemonitor (Server): {error}"))?;
        let deadline = Some(Instant::now() + ADMISSION);
        route_monitor::wait_application_route(&outgoing, deadline)
            .await
            .map_err(|error| format!("Routezulassung (Client): {error}"))?;
        route_monitor::wait_application_route(&accepted, deadline)
            .await
            .map_err(|error| format!("Routezulassung (Server): {error}"))?;
        Ok((outgoing, accepted))
    }

    /// The next incoming connection, accepted with its own server
    /// configuration; the purpose binds from the negotiated ALPN.
    async fn accept_one(&self) -> Result<Connection, String> {
        let incoming = self
            .server
            .accept()
            .await
            .ok_or_else(|| "Server ohne eingehende Verbindung".to_owned())?;
        let (config, slot) = self.profiles.for_incoming(&incoming);
        let connection = incoming
            .accept_with(config)
            .map_err(|error| format!("Annahme: {error}"))?
            .await
            .map_err(|error| format!("Annahme: {error:?}"))?;
        slot.bind_alpn(connection.alpn());
        Ok(connection)
    }

    pub(crate) async fn close(self) {
        self.client.close().await;
        self.server.close().await;
    }
}

/// Closes both sides of a connection.
pub(crate) fn close_pair(pair: &(Connection, Connection)) {
    pair.0.close(0u32.into(), b"messung beendet");
    pair.1.close(0u32.into(), b"messung beendet");
}

/// Writes until `total` bytes are written (`None`: until `stop`), counting.
async fn write_until(
    send: &mut SendStream,
    total: Option<u64>,
    stop: &AtomicBool,
    counter: &AtomicU64,
) -> io::Result<()> {
    let chunk = vec![0x5au8; BLOCK_BYTES];
    let mut sent = 0u64;
    while !stop.load(Ordering::Acquire) && total.is_none_or(|total| sent < total) {
        let count = total.map_or(BLOCK_BYTES as u64, |total| {
            (total - sent).min(BLOCK_BYTES as u64)
        }) as usize;
        send.write_all(&chunk[..count])
            .await
            .map_err(io::Error::other)?;
        sent += count as u64;
        counter.fetch_add(count as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// Reads to the end of the stream, counting; returns the bytes read.
async fn read_count(recv: &mut RecvStream, counter: &AtomicU64) -> io::Result<u64> {
    let mut buffer = vec![0u8; BLOCK_BYTES];
    let mut total = 0u64;
    while let Some(count) = recv.read(&mut buffer).await.map_err(io::Error::other)? {
        total += count as u64;
        counter.fetch_add(count as u64, Ordering::Relaxed);
    }
    Ok(total)
}
