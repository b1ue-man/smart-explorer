//! M1 to M3 (plan 2026-11-14 C7, P7): bulk throughput, constancy of a picture
//! stream next to transfers, and listing latency during a transfer. See
//! `verbindungen_messung_support.rs` for the link and the output lines.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;

use super::{close_pair, median, mib_s, read_count, text, write_until, Link, Tasks};
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::metrics::LatencySamples;
use crate::share::keepalive::interactive_presence::{self, InteractiveKind};
use crate::share::keepalive::metadata_presence::{self, OperationOrigin};
use crate::share::keepalive::transport_purpose::{self, TransportPurpose};

/// Time to let a case settle before it is counted (slow start, queues).
const SETTLE: Duration = Duration::from_secs(3);
/// Length of the counted window of the time-boxed cases (M2, M3).
const WINDOW: Duration = Duration::from_secs(10);
/// Bytes of one counted transfer (1 GiB).
const MEASURED_BYTES: u64 = 1 << 30;
/// Warm-up transfer before the counted ones.
const WARMUP_BYTES: u64 = 64 << 20;
/// Picture stream of M2: 1080p at 60 frames/s, 8 Mbit/s (assumption: a
/// screen picture of this size and rate is of that order; the suite fixes
/// no encoder here).
const PICTURE_FPS: u64 = 60;
const PICTURE_BYTES: usize = 8_000_000 / 8 / PICTURE_FPS as usize;
/// Request and reply of the listing of M3.
const LIST_REPLY_BYTES: usize = 4096;
const LIST_INTERVAL: Duration = Duration::from_millis(50);

/// One stream of `total` bytes from `sender` to `receiver`.
async fn one_stream(sender: &Connection, receiver: &Connection, total: u64) -> Result<(), String> {
    let stop = AtomicBool::new(false);
    let (written, read) = (AtomicU64::new(0), AtomicU64::new(0));
    let sending = async {
        let (mut send, _recv) = sender.open_bi().await.map_err(io::Error::other)?;
        write_until(&mut send, Some(total), &stop, &written).await?;
        send.finish().map_err(io::Error::other)?;
        Ok::<(), io::Error>(())
    };
    let receiving = async {
        let (_send, mut recv) = receiver.accept_bi().await.map_err(io::Error::other)?;
        let received = read_count(&mut recv, &read).await?;
        if received != total {
            return Err(io::Error::other("Strom endet vor dem Ende der Messung"));
        }
        Ok::<(), io::Error>(())
    };
    let (sent, received) = tokio::join!(sending, receiving);
    sent.map_err(|error| format!("Senden: {error}"))?;
    received.map_err(|error| format!("Empfangen: {error}"))
}

/// `total` bytes in all, split evenly over `pairs`, one stream each at once.
async fn transfer_all(pairs: &[(Connection, Connection)], total: u64) -> Result<(), String> {
    let share = total / pairs.len().max(1) as u64;
    let mut handles = Vec::new();
    for (client, server) in pairs {
        let (client, server) = (client.clone(), server.clone());
        handles.push(tokio::spawn(async move {
            one_stream(&client, &server, share).await
        }));
    }
    for handle in handles {
        handle.await.map_err(text)??;
    }
    Ok(())
}

/// Time the three counted runs of M1 may take together; setup, warm-up and
/// closing take the rest of the case budget of 60 s.
const COUNTED_RUNS: Duration = Duration::from_secs(42);

pub(crate) struct BulkResult {
    /// Median of the three counted runs.
    pub mib_s: f64,
    /// MiB of one counted run (1024 unless the runner is too slow for it).
    pub counted_mib: u64,
}

/// Bytes of one counted run on a link that moved the warm-up at
/// `warmup_mib_s`: 1 GiB, or, where three runs of 1 GiB do not fit into
/// `COUNTED_RUNS` at that rate, the whole multiple of the warm-up size that
/// does (at least one). A slow runner then still measures and reports the
/// size it counted, instead of ending at the case budget without a line.
fn counted_bytes(warmup_mib_s: f64) -> u64 {
    let per_run = warmup_mib_s * COUNTED_RUNS.as_secs_f64() / 3.0 * 1024.0 * 1024.0;
    let steps = (per_run / WARMUP_BYTES as f64).floor().max(1.0) as u64;
    MEASURED_BYTES.min(steps.saturating_mul(WARMUP_BYTES))
}

/// M1: throughput of `connections` connections of the `Files` profile,
/// 1 GiB in all per counted run, in MiB/s (median of three after a warm-up;
/// `counted_bytes` for a runner too slow for that). The send batch and the
/// ACK frequency come from the switches the caller holds.
pub(crate) async fn bulk_throughput(connections: usize) -> Result<BulkResult, String> {
    let link = Link::new().await?;
    let mut pairs = Vec::new();
    for _ in 0..connections.max(1) {
        pairs.push(
            link.connect(crate::share::node::ALPN, TransportPurpose::Files)
                .await?,
        );
    }
    let warmup = Instant::now();
    transfer_all(&pairs, WARMUP_BYTES).await?;
    let counted = counted_bytes(mib_s(WARMUP_BYTES, warmup.elapsed()));
    let mut runs = [0.0; 3];
    for run in &mut runs {
        let started = Instant::now();
        transfer_all(&pairs, counted).await?;
        *run = mib_s(counted, started.elapsed());
    }
    pairs.iter().for_each(close_pair);
    link.close().await;
    Ok(BulkResult {
        mib_s: median(runs),
        counted_mib: counted >> 20,
    })
}

/// What one M2 case runs at the same time.
#[derive(Clone, Copy)]
pub(crate) struct ConstancyCase {
    /// The picture stream (1080p/60).
    pub picture: bool,
    /// A download and an upload of the `Files` profile to the same peer.
    pub transfer: bool,
    /// An interactive session is registered (as the screen host does).
    pub presence: bool,
}

pub(crate) struct ConstancyResult {
    pub frames: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub bulk_mib_s: f64,
}

/// The picture sender: one frame of `PICTURE_BYTES` per tick behind a time stamp.
async fn picture_sender(connection: Connection, stop: Arc<AtomicBool>) {
    let Ok((mut send, _recv)) = connection.open_bi().await else {
        return;
    };
    let payload = vec![0x3cu8; PICTURE_BYTES];
    let mut tick = tokio::time::interval(Duration::from_micros(1_000_000 / PICTURE_FPS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !stop.load(Ordering::Acquire) {
        tick.tick().await;
        let stamp = now_us().to_be_bytes();
        if send.write_all(&stamp).await.is_err() || send.write_all(&payload).await.is_err() {
            return;
        }
    }
}

/// The picture reader: send-to-complete time of every frame while `measuring`.
async fn picture_receiver(
    connection: Connection,
    measuring: Arc<AtomicBool>,
    latency: Arc<Mutex<LatencySamples>>,
) {
    let Ok((_send, mut recv)) = connection.accept_bi().await else {
        return;
    };
    let mut frame = vec![0u8; 8 + PICTURE_BYTES];
    while recv.read_exact(&mut frame).await.is_ok() {
        let stamp: [u8; 8] = frame[..8].try_into().unwrap_or([0; 8]);
        let sent = u64::from_be_bytes(stamp);
        if measuring.load(Ordering::Acquire) {
            if let Ok(mut samples) = latency.lock() {
                samples.record(now_us().saturating_sub(sent));
            }
        }
    }
}

/// Writes on a new stream until stopped; the stream gets its transfer
/// priority first, as `open_transfer_stream_until` does.
async fn bulk_writer(connection: Connection, stop: Arc<AtomicBool>, tag: Option<u8>) {
    let Ok((mut send, _recv)) = connection.open_bi().await else {
        return;
    };
    transport_purpose::prioritize_transfer(&send);
    if let Some(tag) = tag {
        if send.write_all(&[tag]).await.is_err() {
            return;
        }
    }
    let written = AtomicU64::new(0);
    let _ = write_until(&mut send, None, &stop, &written).await;
}

/// Reads a new incoming stream to its end, counting into `counter`.
async fn bulk_reader(connection: Connection, counter: Arc<AtomicU64>) {
    let Ok((_send, mut recv)) = connection.accept_bi().await else {
        return;
    };
    let _ = read_count(&mut recv, &counter).await;
}

/// M2: the picture stream with and without a download and an upload at the
/// same time, over a `Screen` and a `Files` connection to the same peer.
/// The caller holds the switches (controller, target `T`).
pub(crate) async fn constancy(case: ConstancyCase) -> Result<ConstancyResult, String> {
    let link = Link::new().await?;
    let screen = link
        .connect(
            crate::screen::core::wire::SCREEN_ALPN,
            TransportPurpose::Screen,
        )
        .await?;
    let files = link
        .connect(crate::share::node::ALPN, TransportPurpose::Files)
        .await?;
    let _presence = case
        .presence
        .then(|| interactive_presence::enter(InteractiveKind::ScreenHost));
    let stop = Arc::new(AtomicBool::new(false));
    let measuring = Arc::new(AtomicBool::new(false));
    let latency = Arc::new(Mutex::new(LatencySamples::default()));
    let (uploaded, downloaded) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let mut tasks = Tasks::default();
    if case.picture {
        tasks.spawn(picture_sender(screen.0.clone(), stop.clone()));
        tasks.spawn(picture_receiver(
            screen.1.clone(),
            measuring.clone(),
            latency.clone(),
        ));
    }
    if case.transfer {
        tasks.spawn(bulk_writer(files.0.clone(), stop.clone(), None));
        tasks.spawn(bulk_reader(files.1.clone(), uploaded.clone()));
        tasks.spawn(bulk_writer(files.1.clone(), stop.clone(), None));
        tasks.spawn(bulk_reader(files.0.clone(), downloaded.clone()));
    }
    tokio::time::sleep(SETTLE).await;
    let before = uploaded.load(Ordering::Relaxed) + downloaded.load(Ordering::Relaxed);
    measuring.store(true, Ordering::Release);
    let started = Instant::now();
    tokio::time::sleep(WINDOW).await;
    let elapsed = started.elapsed();
    measuring.store(false, Ordering::Release);
    let after = uploaded.load(Ordering::Relaxed) + downloaded.load(Ordering::Relaxed);
    stop.store(true, Ordering::Release);
    drop(tasks);
    close_pair(&screen);
    close_pair(&files);
    link.close().await;
    if case.transfer && after <= before {
        return Err("die Uebertragung hat im Messfenster keine Daten bewegt".to_owned());
    }
    let samples = latency
        .lock()
        .map_err(|_| "Messwerte gesperrt".to_owned())?;
    let (frames, p50_ms, p95_ms) = match samples.summary() {
        Some(summary) => (samples.count(), summary.p50_ms, summary.p95_ms),
        None if case.picture => return Err("kein Bild angekommen".to_owned()),
        None => (0, 0.0, 0.0),
    };
    Ok(ConstancyResult {
        frames,
        p50_ms,
        p95_ms,
        bulk_mib_s: mib_s(after.saturating_sub(before), elapsed),
    })
}

/// The peer of M3: answers listing requests (`L`) and swallows uploads (`U`).
/// A listing request sets the host's metadata presence for the asking
/// device with the call the host makes for a request marked `user`
/// (`server.rs`), and `replies` takes the time its reply needed to be
/// accepted into the send buffer.
async fn browse_server(
    connection: Connection,
    uploaded: Arc<AtomicU64>,
    replies: Arc<Mutex<LatencySamples>>,
) {
    let asker = connection.remote_id();
    while let Ok((mut send, mut recv)) = connection.accept_bi().await {
        let uploaded = uploaded.clone();
        let replies = replies.clone();
        tokio::spawn(async move {
            let mut tag = [0u8; 1];
            if recv.read_exact(&mut tag).await.is_err() {
                return;
            }
            match tag[0] {
                b'U' => {
                    let _ = read_count(&mut recv, &uploaded).await;
                }
                b'L' => {
                    let mut request = [0u8; 8];
                    if recv.read_exact(&mut request).await.is_err() {
                        return;
                    }
                    let _presence = metadata_presence::begin(asker, OperationOrigin::User);
                    let reply = [0x4cu8; LIST_REPLY_BYTES];
                    let started = Instant::now();
                    if send.write_all(&reply).await.is_ok() && send.finish().is_ok() {
                        if let Ok(mut samples) = replies.lock() {
                            samples.record(micros(started.elapsed()));
                        }
                    }
                }
                _ => {}
            }
        });
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// One listing request: 8 bytes out, `LIST_REPLY_BYTES` back, on a new
/// stream, under the client's metadata presence for the host (the call
/// `PeerBackend::metadata_presence` makes). Returns the time until the
/// request was accepted into the send buffer.
async fn list_once(connection: &Connection) -> Result<Duration, String> {
    let _presence = metadata_presence::begin(connection.remote_id(), OperationOrigin::User);
    let started = Instant::now();
    let (mut send, mut recv) = connection.open_bi().await.map_err(text)?;
    send.write_all(&[b'L']).await.map_err(text)?;
    send.write_all(&now_us().to_be_bytes())
        .await
        .map_err(text)?;
    send.finish().map_err(text)?;
    let accepted = started.elapsed();
    let reply = recv.read_to_end(2 * LIST_REPLY_BYTES).await.map_err(text)?;
    if reply.len() != LIST_REPLY_BYTES {
        return Err(format!("Antwort mit {} Bytes", reply.len()));
    }
    Ok(accepted)
}

/// The smoothed round trip of the selected path, read as
/// `quic_stats_hub.rs` reads it.
fn smoothed_rtt(connection: &Connection) -> Option<Duration> {
    let paths = connection.paths();
    let selected = paths
        .iter()
        .find(|path| path.is_selected())
        .or_else(|| paths.iter().next());
    selected.map(|path| path.stats().rtt)
}

pub(crate) struct BrowsingResult {
    pub requests: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub bulk_mib_s: f64,
    /// p95 of the time until a request was accepted into the client's send
    /// buffer, and the same for the host's reply.
    pub request_accept_p95_ms: f64,
    pub reply_accept_p95_ms: f64,
    /// Median smoothed round trip after each request, client and host.
    pub client_srtt_ms: f64,
    pub host_srtt_ms: f64,
    /// Packets each side lost in the counted window (`lost_packets`).
    pub client_lost: u64,
    pub host_lost: u64,
}

fn p95(samples: &LatencySamples) -> f64 {
    samples.summary().map_or(0.0, |summary| summary.p95_ms)
}

fn p50(samples: &LatencySamples) -> f64 {
    samples.summary().map_or(0.0, |summary| summary.p50_ms)
}

/// M3: listing latency on a file session connection while it transfers in
/// both directions. Every listing sets the metadata presence on both sides
/// as the product does (`OperationOrigin::User`); the caller holds the
/// switches (priority of transfer streams, yielding controller), so the
/// presence has an effect only where yielding is on.
pub(crate) async fn browsing() -> Result<BrowsingResult, String> {
    let link = Link::new().await?;
    let files = link
        .connect(crate::share::node::ALPN, TransportPurpose::Files)
        .await?;
    let stop = Arc::new(AtomicBool::new(false));
    let (uploaded, downloaded) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let replies = Arc::new(Mutex::new(LatencySamples::default()));
    let mut tasks = Tasks::default();
    tasks.spawn(browse_server(
        files.1.clone(),
        uploaded.clone(),
        replies.clone(),
    ));
    tasks.spawn(bulk_writer(files.0.clone(), stop.clone(), Some(b'U')));
    tasks.spawn(bulk_writer(files.1.clone(), stop.clone(), None));
    tasks.spawn(bulk_reader(files.0.clone(), downloaded.clone()));
    tokio::time::sleep(SETTLE).await;
    let before = uploaded.load(Ordering::Relaxed) + downloaded.load(Ordering::Relaxed);
    let lost_before = (files.0.stats().lost_packets, files.1.stats().lost_packets);
    let started = Instant::now();
    let mut samples = LatencySamples::default();
    let mut accepted = LatencySamples::default();
    let (mut client_srtt, mut host_srtt) = (LatencySamples::default(), LatencySamples::default());
    while started.elapsed() < WINDOW {
        let asked = Instant::now();
        accepted.record(micros(list_once(&files.0).await?));
        samples.record(micros(asked.elapsed()));
        if let Some(rtt) = smoothed_rtt(&files.0) {
            client_srtt.record(micros(rtt));
        }
        if let Some(rtt) = smoothed_rtt(&files.1) {
            host_srtt.record(micros(rtt));
        }
        tokio::time::sleep(LIST_INTERVAL).await;
    }
    let elapsed = started.elapsed();
    let after = uploaded.load(Ordering::Relaxed) + downloaded.load(Ordering::Relaxed);
    let lost = (
        files.0.stats().lost_packets.saturating_sub(lost_before.0),
        files.1.stats().lost_packets.saturating_sub(lost_before.1),
    );
    stop.store(true, Ordering::Release);
    drop(tasks);
    close_pair(&files);
    link.close().await;
    if after <= before {
        return Err("die Uebertragung hat im Messfenster keine Daten bewegt".to_owned());
    }
    let summary = samples
        .summary()
        .ok_or_else(|| "keine Listenantwort".to_owned())?;
    let reply_accept_p95_ms = replies.lock().map_or(0.0, |replies| p95(&replies));
    Ok(BrowsingResult {
        requests: samples.count(),
        p50_ms: summary.p50_ms,
        p95_ms: summary.p95_ms,
        bulk_mib_s: mib_s(after.saturating_sub(before), elapsed),
        request_accept_p95_ms: p95(&accepted),
        reply_accept_p95_ms,
        client_srtt_ms: p50(&client_srtt),
        host_srtt_ms: p50(&host_srtt),
        client_lost: lost.0,
        host_lost: lost.1,
    })
}
