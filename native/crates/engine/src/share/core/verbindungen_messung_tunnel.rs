//! M4 and M8 (plan 2026-11-14 C7, P7): one tunnel of the `Tcp` profile, and
//! the cost of one connection per short `Exec` job. See
//! `verbindungen_messung_support.rs` for the link and the output lines.
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{close_pair, mib_s, text, Link, BLOCK_BYTES};
use crate::screen::core::metrics::LatencySamples;
use crate::share::keepalive::transport_purpose::TransportPurpose;
use crate::share::route_protocol;
use crate::share::tcp_registry::TcpCounters;

/// Bytes of the counted transfer (1 GiB).
const MEASURED_BYTES: u64 = 1 << 30;
/// Copy buffer and per-operation budget of the copy mode before plan E7.
const LEGACY_COPY_BYTES: usize = 8 * 1024;
const LEGACY_OPERATION_BUDGET: Duration = Duration::from_secs(86_400);

/// The copy mode before plan E7 (the test switch `set_legacy_copy` of
/// `tcp_copy_budget.rs` is private to `tcp_pump.rs`): 8 KiB per call and one
/// route wrapper per operation (`PumpRoute::Legacy`).
async fn legacy_pump(
    connection: &Connection,
    socket: &mut TcpStream,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> io::Result<()> {
    let (mut read, mut write) = socket.split();
    let up = async {
        let mut buffer = vec![0u8; LEGACY_COPY_BYTES];
        loop {
            let length = route_protocol::run_with_route(
                connection,
                LEGACY_OPERATION_BUDGET,
                None,
                read.read(&mut buffer),
            )
            .await?;
            if length == 0 {
                send.finish().map_err(io::Error::other)?;
                return Ok::<(), io::Error>(());
            }
            route_protocol::run_with_route(connection, LEGACY_OPERATION_BUDGET, None, async {
                send.write_all(&buffer[..length])
                    .await
                    .map_err(io::Error::other)
            })
            .await?;
        }
    };
    let down = async {
        let mut buffer = vec![0u8; LEGACY_COPY_BYTES];
        loop {
            let length =
                route_protocol::run_with_route(connection, LEGACY_OPERATION_BUDGET, None, async {
                    recv.read(&mut buffer).await.map_err(io::Error::other)
                })
                .await?;
            match length {
                None => {
                    route_protocol::run_with_route(
                        connection,
                        LEGACY_OPERATION_BUDGET,
                        None,
                        write.shutdown(),
                    )
                    .await?;
                    return Ok::<(), io::Error>(());
                }
                Some(count) => {
                    route_protocol::run_with_route(
                        connection,
                        LEGACY_OPERATION_BUDGET,
                        None,
                        write.write_all(&buffer[..count]),
                    )
                    .await?;
                }
            }
        }
    };
    tokio::try_join!(up, down).map(|_| ())
}

/// One pump end: the production pump, or the copy mode before E7.
async fn pump_end(
    legacy: bool,
    connection: &Connection,
    socket: &mut TcpStream,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> io::Result<()> {
    if legacy {
        return legacy_pump(connection, socket, send, recv).await;
    }
    route_protocol::scope(
        connection.clone(),
        crate::share::tcp_pump::pump(
            connection,
            socket,
            send,
            recv,
            Arc::new(TcpCounters::default()),
        ),
    )
    .await
}

/// M4: 1 GiB through one tunnel of the `Tcp` profile (a local socket, a pump
/// at each end, one QUIC stream between them), in MiB/s. `legacy`: the copy
/// mode before E7 at both ends.
pub(crate) async fn tunnel_throughput(legacy: bool) -> Result<f64, String> {
    let link = Link::new().await?;
    let (client, target) = link
        .connect(crate::share::tcp_forward::TCP_ALPN, TransportPurpose::Tcp)
        .await?;
    let near = TcpListener::bind("127.0.0.1:0").await.map_err(text)?;
    let far = TcpListener::bind("127.0.0.1:0").await.map_err(text)?;
    let (near_addr, far_addr) = (
        near.local_addr().map_err(text)?,
        far.local_addr().map_err(text)?,
    );
    let (producer, near_accepted) = tokio::join!(TcpStream::connect(near_addr), near.accept());
    let mut producer = producer.map_err(text)?;
    let (mut near_socket, _) = near_accepted.map_err(text)?;
    let (target_socket, far_accepted) = tokio::join!(TcpStream::connect(far_addr), far.accept());
    let mut target_socket = target_socket.map_err(text)?;
    let (mut consumer, _) = far_accepted.map_err(text)?;
    let (mut client_send, mut client_recv) = client.open_bi().await.map_err(text)?;
    let started = Instant::now();
    let client_side = pump_end(
        legacy,
        &client,
        &mut near_socket,
        &mut client_send,
        &mut client_recv,
    );
    let target_side = async {
        let (mut send, mut recv) = target.accept_bi().await.map_err(io::Error::other)?;
        pump_end(legacy, &target, &mut target_socket, &mut send, &mut recv).await
    };
    let producing = async {
        let chunk = vec![0x5au8; BLOCK_BYTES];
        let mut sent = 0u64;
        while sent < MEASURED_BYTES {
            let count = (MEASURED_BYTES - sent).min(BLOCK_BYTES as u64) as usize;
            producer.write_all(&chunk[..count]).await?;
            sent += count as u64;
        }
        producer.shutdown().await?;
        let mut rest = Vec::new();
        producer.read_to_end(&mut rest).await?;
        Ok::<(), io::Error>(())
    };
    let consuming = async {
        let mut buffer = vec![0u8; BLOCK_BYTES];
        let mut received = 0u64;
        loop {
            let count = consumer.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            received += count as u64;
        }
        let elapsed = started.elapsed();
        consumer.shutdown().await?;
        Ok::<(u64, Duration), io::Error>((received, elapsed))
    };
    let (client_done, target_done, produced, consumed) =
        tokio::join!(client_side, target_side, producing, consuming);
    client_done.map_err(|error| format!("Pumpe (Client): {error}"))?;
    target_done.map_err(|error| format!("Pumpe (Ziel): {error}"))?;
    produced.map_err(|error| format!("Erzeuger: {error}"))?;
    let (received, elapsed) = consumed.map_err(|error| format!("Verbraucher: {error}"))?;
    if received != MEASURED_BYTES {
        return Err(format!("{received} von {MEASURED_BYTES} Bytes angekommen"));
    }
    close_pair(&(client, target));
    link.close().await;
    Ok(mib_s(received, elapsed))
}

/// One request and its answer on a new stream of the `Exec` connection.
async fn exec_round(client: &Connection, server: &Connection) -> Result<(), String> {
    let asking = async {
        let (mut send, mut recv) = client.open_bi().await.map_err(io::Error::other)?;
        send.write_all(b"true").await.map_err(io::Error::other)?;
        send.finish().map_err(io::Error::other)?;
        let reply = recv.read_to_end(64).await.map_err(io::Error::other)?;
        if reply.as_slice() != b"done" {
            return Err(io::Error::other("unerwartete Antwort"));
        }
        Ok::<(), io::Error>(())
    };
    let answering = async {
        let (mut send, mut recv) = server.accept_bi().await.map_err(io::Error::other)?;
        recv.read_to_end(64).await.map_err(io::Error::other)?;
        send.write_all(b"done").await.map_err(io::Error::other)?;
        send.finish().map_err(io::Error::other)?;
        Ok::<(), io::Error>(())
    };
    let (asked, answered) = tokio::join!(asking, answering);
    asked.map_err(|error| format!("Anfrage: {error}"))?;
    answered.map_err(|error| format!("Antwort: {error}"))
}

pub(crate) struct ExecSetupResult {
    pub jobs: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub total_ms: f64,
}

/// M8: `jobs` short jobs on the `Exec` profile. `fresh`: each job makes its
/// own connection (as `node_routes.rs` does: handshake, admission, request,
/// answer, close); otherwise all jobs share one connection and pay only the
/// request on a new stream. The connection setup of the shared variant is
/// not counted. Lower bound of the real Exec cost: no hello, no session
/// proof, no rights check.
pub(crate) async fn exec_setup(fresh: bool, jobs: usize) -> Result<ExecSetupResult, String> {
    let link = Link::new().await?;
    let shared = if fresh {
        None
    } else {
        Some(
            link.connect(
                crate::share::exec_protocol::EXEC_ALPN,
                TransportPurpose::Exec,
            )
            .await?,
        )
    };
    let mut samples = LatencySamples::default();
    let all = Instant::now();
    for _ in 0..jobs {
        let started = Instant::now();
        match &shared {
            Some(pair) => exec_round(&pair.0, &pair.1).await?,
            None => {
                let pair = link
                    .connect(
                        crate::share::exec_protocol::EXEC_ALPN,
                        TransportPurpose::Exec,
                    )
                    .await?;
                exec_round(&pair.0, &pair.1).await?;
                close_pair(&pair);
            }
        }
        samples.record(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    let total = all.elapsed();
    if let Some(pair) = &shared {
        close_pair(pair);
    }
    link.close().await;
    let summary = samples
        .summary()
        .ok_or_else(|| "keine Messwerte".to_owned())?;
    Ok(ExecSetupResult {
        jobs,
        p50_ms: summary.p50_ms,
        p95_ms: summary.p95_ms,
        total_ms: total.as_secs_f64() * 1000.0,
    })
}
