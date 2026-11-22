//! Host side of the sound (plan 2026-11-17 P9 „Audio“): one link per
//! session connection that answered `audio` in `HELLO_OK`. While the viewer
//! wants the sound (`AUDIO` on) the link runs a capture process and sends
//! its packets as datagrams of the connection; `AUDIO` off ends the
//! process, so a muted session costs nothing on the host. A capture process
//! that ends (default device changed, sound server restarted) starts again
//! while the viewer still wants the sound, after `RESTART_PAUSE`, doubled
//! after each process that ended without a packet up to `MAX_PAUSE` (no
//! sound server in the session). A datagram the connection cannot take
//! (path without datagrams, full send buffer) is dropped: sound is never
//! queued behind the picture.
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use iroh::endpoint::Connection;
use tokio::sync::{mpsc, watch};

use crate::screen::core::audio_codec::{opus_bitrate_for, sound_fits};
use crate::screen::core::audio_wire::read_piped;

/// Environment variable that makes `--screen-host-worker` the capture
/// process.
pub(super) const HELPER_ENV: &str = "SE_SCREEN_AUDIO_HOST";
/// Wait before a capture process that ended is started again.
const RESTART_PAUSE: Duration = Duration::from_secs(1);
/// Longest wait between attempts that deliver nothing.
const MAX_PAUSE: Duration = Duration::from_secs(30);
/// Packets between the capture process's reader and the sender (320 ms).
const PACKET_QUEUE: usize = 64;

/// One attached connection: its `Connection::stable_id`, the viewer's
/// wish, the bytes of sound sent on it (`AudioHost::sent_bytes`) and the
/// viewer's „Höchste Datenrate“ (`AudioHost::set_ceiling`).
struct Link {
    id: usize,
    wanted: watch::Sender<bool>,
    sent: Arc<AtomicU64>,
    ceiling: watch::Sender<Option<u64>>,
}

/// The attached connections.
static LINKS: Mutex<Vec<Link>> = Mutex::new(Vec::new());

fn links() -> std::sync::MutexGuard<'static, Vec<Link>> {
    LINKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The port the session reaches this link through (`core/audio_port.rs`).
pub(super) struct CaptureLink;

impl crate::screen::core::audio_port::AudioHost for CaptureLink {
    fn attach(&self, connection: &Connection) {
        attach(connection);
    }

    fn set_wanted(&self, connection: &Connection, on: bool) {
        let id = connection.stable_id();
        if let Some(link) = links().iter().find(|link| link.id == id) {
            link.wanted.send_replace(on);
        }
    }

    fn sent_bytes(&self, connection: &Connection) -> u64 {
        let id = connection.stable_id();
        links()
            .iter()
            .find(|link| link.id == id)
            .map_or(0, |link| link.sent.load(Ordering::Acquire))
    }

    fn set_ceiling(&self, connection: &Connection, ceiling_bps: Option<u64>) {
        let id = connection.stable_id();
        if let Some(link) = links().iter().find(|link| link.id == id) {
            link.ceiling.send_replace(ceiling_bps);
        }
    }
}

/// Attaches the sound link to a session's connection (after `HELLO_OK`
/// with `audio`); it ends with the connection.
fn attach(connection: &Connection) {
    let (wanted, watched) = watch::channel(false);
    let (ceiling, ceiling_watched) = watch::channel(None);
    let id = connection.stable_id();
    let sent = Arc::new(AtomicU64::new(0));
    links().push(Link {
        id,
        wanted,
        sent: sent.clone(),
        ceiling,
    });
    let connection = connection.clone();
    tokio::spawn(async move {
        run(&connection, watched, ceiling_watched, &sent).await;
        links().retain(|link| link.id != id);
    });
}

async fn run(
    connection: &Connection,
    mut wanted: watch::Receiver<bool>,
    mut ceiling: watch::Receiver<Option<u64>>,
    sent: &AtomicU64,
) {
    let mut failures = 0u32;
    loop {
        // Until the viewer wants the sound.
        while !*wanted.borrow_and_update() {
            tokio::select! {
                changed = wanted.changed() => if changed.is_err() { return },
                _ = connection.closed() => return,
            }
        }
        let started = tokio::task::spawn_blocking(Capture::start).await;
        let mut capture = match started {
            Ok(Ok(capture)) => capture,
            Ok(Err(error)) => {
                eprintln!("screen audio: capture process did not start: {error}");
                failures = failures.saturating_add(1);
                if !pause(connection, &mut wanted, failures).await {
                    return;
                }
                continue;
            }
            Err(_) => return,
        };
        // The capture process codes at the rate the ceiling leaves the sound;
        // under a ceiling the sound does not fit, none is sent (round 8
        // Gemini 8/10: the session tells the viewer why).
        let mut fits = sound_fits(*ceiling.borrow());
        capture.set_rate(*ceiling.borrow_and_update());
        loop {
            tokio::select! {
                packet = capture.packets.recv() => match packet {
                    // Dropped when the connection cannot take it now.
                    Some(_) if !fits => failures = 0,
                    Some(packet) => {
                        failures = 0;
                        let bytes = packet.len() as u64;
                        // Counted against „Höchste Datenrate“ (`sent_bytes`).
                        if connection.send_datagram(Bytes::from(packet)).is_ok() {
                            sent.fetch_add(bytes, Ordering::AcqRel);
                        }
                    }
                    None => {
                        failures = failures.saturating_add(1);
                        break;
                    }
                },
                changed = wanted.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if !*wanted.borrow_and_update() {
                        break;
                    }
                }
                changed = ceiling.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    fits = sound_fits(*ceiling.borrow());
                    capture.set_rate(*ceiling.borrow_and_update());
                }
                _ = connection.closed() => return,
            }
        }
        drop(capture);
        let still_wanted = *wanted.borrow();
        if still_wanted && !pause(connection, &mut wanted, failures).await {
            return;
        }
    }
}

/// The wait after `failures` attempts in a row that delivered nothing.
fn restart_pause(failures: u32) -> Duration {
    let doubled = RESTART_PAUSE.saturating_mul(1u32 << failures.saturating_sub(1).min(5));
    doubled.min(MAX_PAUSE)
}

/// Waits before the next attempt; `false` when the connection closed
/// meanwhile. A change of the wish ends the wait early.
async fn pause(connection: &Connection, wanted: &mut watch::Receiver<bool>, failures: u32) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(restart_pause(failures)) => true,
        changed = wanted.changed() => changed.is_ok(),
        _ = connection.closed() => false,
    }
}

/// A running capture process and its packets.
struct Capture {
    child: Child,
    /// Held open: the process ends at the end of its input.
    input: Option<ChildStdin>,
    packets: mpsc::Receiver<Vec<u8>>,
}

impl Capture {
    fn start() -> io::Result<Self> {
        let program = program()?;
        let mut command = Command::new(program);
        super::platform::prepare_helper(&mut command);
        command
            .arg(crate::screen::os::host_worker_client::SCREEN_HOST_WORKER_SWITCH)
            .env(HELPER_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn()?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("audio capture process has no pipes"));
        };
        let (sender, packets) = mpsc::channel(PACKET_QUEUE);
        let reader = std::thread::Builder::new()
            .name("screen-audio-reader".into())
            .spawn(move || {
                let mut output = BufReader::new(output);
                while let Ok(Some(packet)) = read_piped(&mut output) {
                    // A full queue drops the packet: the sender is behind.
                    if let Err(mpsc::error::TrySendError::Closed(_)) = sender.try_send(packet) {
                        return;
                    }
                }
            });
        if let Err(error) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            child,
            input: Some(input),
            packets,
        })
    }
}

impl Capture {
    /// Hands the capture process the Opus rate under `ceiling_bps`
    /// (`audio_helper`'s input: 4 bytes big-endian). A process that does
    /// not take it has ended; its packets end and it starts again.
    fn set_rate(&mut self, ceiling_bps: Option<u64>) {
        let rate = opus_bitrate_for(ceiling_bps).to_be_bytes();
        if let Some(input) = self.input.as_mut() {
            let _ = input.write_all(&rate).and_then(|()| input.flush());
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // The end of its input ends the process; kill what is left.
        self.input.take();
        self.packets.close();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The program of the capture process: the desktop program next to this
/// one, or this program.
fn program() -> io::Result<PathBuf> {
    use crate::screen::os::host_worker_client::WorkerProgram;
    if let WorkerProgram::Installation {
        gui: Some(path), ..
    } = WorkerProgram::of_this_installation()
    {
        return Ok(path);
    }
    std::env::current_exe()
}
