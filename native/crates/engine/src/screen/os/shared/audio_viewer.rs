//! Viewer side of the host's sound (plan 2026-11-17 P9 „Audio“).
//!
//! The daemon that holds the viewer's connection reads its datagrams and
//! forwards the sound packets to the viewer window's process over the relay
//! (`relay`, `RELAY_AUDIO`); a full relay queue drops a packet rather than
//! delay a picture. The window's process plays them: `deliver` puts each
//! packet into the jitter buffer (`core/audio_jitter.rs`) and starts the
//! output thread, which opens the default output device, takes one frame
//! from the buffer whenever the device has room for it, decodes it (a lost
//! frame is concealed by the decoder) and writes it; while the buffer has
//! nothing it writes silence, so the device never runs dry and its delay
//! stays constant. The thread closes the device after `IDLE_CLOSE` without
//! a packet or when the viewer switches the sound off. One player per
//! process: a viewer window shows one full session (the group view shows
//! tiles, which never switch the sound on).
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;
use tokio::sync::mpsc;

use crate::screen::core::audio_codec::{AudioDecoder, FRAME_VALUES};
use crate::screen::core::audio_jitter::{JitterBuffer, Playout};
use crate::screen::core::audio_wire::{AudioCodec, AudioPacket, AudioReport, SAMPLE_RATE};

/// The output closes after this long without a packet.
const IDLE_CLOSE: Duration = Duration::from_secs(2);

type Frame = (AudioCodec, Vec<u8>);

struct Player {
    jitter: Mutex<JitterBuffer<Frame>>,
    /// The viewer's „Ton“.
    playing: AtomicBool,
    /// The output thread runs.
    running: AtomicBool,
    last_packet: Mutex<Option<Instant>>,
    epoch: Instant,
    /// What the output holds ahead of the speaker (device queue and the
    /// decoded rest of a frame), in µs.
    output_us: AtomicU32,
}

fn player() -> &'static Player {
    static PLAYER: OnceLock<Player> = OnceLock::new();
    PLAYER.get_or_init(|| Player {
        jitter: Mutex::new(JitterBuffer::default()),
        playing: AtomicBool::new(false),
        running: AtomicBool::new(false),
        last_packet: Mutex::new(None),
        epoch: Instant::now(),
        output_us: AtomicU32::new(0),
    })
}

impl Player {
    fn jitter(&self) -> MutexGuard<'_, JitterBuffer<Frame>> {
        self.jitter.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn idle(&self) -> bool {
        self.last_packet
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none_or(|last| last.elapsed() > IDLE_CLOSE)
    }
}

/// The player's statistics (`AUDIO_REPORT`, the viewer's statistics);
/// `None` while „Ton“ is off or the output does not run.
pub fn report() -> Option<AudioReport> {
    let player = player();
    let playing = player.playing.load(Ordering::Acquire) && player.running.load(Ordering::Acquire);
    playing.then(|| {
        player
            .jitter()
            .report(player.output_us.load(Ordering::Relaxed))
    })
}

/// The viewer's „Ton“: off forgets every buffered frame and closes the
/// output.
pub fn set_playing(on: bool) {
    let player = player();
    player.playing.store(on, Ordering::Release);
    if !on {
        player.jitter().clear();
    }
}

/// One datagram of the session (`RELAY_AUDIO`); ignored while „Ton“ is off
/// and when it is no sound packet of a known codec.
pub fn deliver(datagram: &[u8]) {
    let player = player();
    if !player.playing.load(Ordering::Acquire) {
        return;
    }
    let Ok(packet) = AudioPacket::decode(datagram) else {
        return;
    };
    if packet.codec == AudioCodec::Other {
        return;
    }
    let arrival_us = u64::try_from(player.epoch.elapsed().as_micros()).unwrap_or(u64::MAX);
    player.jitter().push(
        packet.sequence,
        packet.host_us,
        arrival_us,
        (packet.codec, packet.payload),
    );
    *player
        .last_packet
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    if player.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let started = std::thread::Builder::new()
        .name("screen-audio-output".into())
        .spawn(|| {
            let player = self::player();
            if let Err(error) = play(player) {
                eprintln!("screen audio: output: {error}");
            }
            player.running.store(false, Ordering::Release);
        });
    if started.is_err() {
        player.running.store(false, Ordering::Release);
    }
}

fn play(player: &Player) -> io::Result<()> {
    let mut output = super::platform::Output::open()?;
    let mut decoder: Option<AudioDecoder> = None;
    let mut frame = vec![0.0f32; FRAME_VALUES];
    let mut pcm: Vec<f32> = Vec::new();
    while player.playing.load(Ordering::Acquire) && !player.idle() {
        let wanted = output.wait_space()? * 2;
        while pcm.len() < wanted {
            let next = player.jitter().pop();
            match next {
                Playout::Frame((codec, payload)) => {
                    if decoder.as_ref().is_none_or(|known| known.codec() != codec) {
                        decoder = Some(AudioDecoder::new(codec)?);
                    }
                    if let Some(decoder) = decoder.as_mut() {
                        decoder.decode(Some(payload.as_slice()), &mut frame)?;
                    }
                }
                Playout::Lost => match decoder.as_mut() {
                    Some(decoder) => decoder.decode(None, &mut frame)?,
                    None => frame.fill(0.0),
                },
                Playout::Silence => frame.fill(0.0),
            }
            pcm.extend_from_slice(&frame);
        }
        if wanted > 0 {
            output.write(&pcm[..wanted])?;
            pcm.drain(..wanted);
        }
        let rest_us = (pcm.len() / 2) as u64 * 1_000_000 / u64::from(SAMPLE_RATE);
        let ahead = u64::from(output.queued_us()).saturating_add(rest_us);
        player
            .output_us
            .store(u32::try_from(ahead).unwrap_or(u32::MAX), Ordering::Relaxed);
    }
    player.output_us.store(0, Ordering::Relaxed);
    Ok(())
}

/// Ends the forwarding of a relayed session's sound when dropped.
pub struct RelayGuard(tokio::task::JoinHandle<()>);

impl Drop for RelayGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Daemon of the viewer: forwards the sound datagrams of `connection` to
/// the viewer window as `(tag, packet)`; a full queue drops the packet.
pub fn relay(connection: &Connection, out: &mpsc::Sender<(u8, Vec<u8>)>, tag: u8) -> RelayGuard {
    let connection = connection.clone();
    let out = out.clone();
    RelayGuard(tokio::spawn(async move {
        while let Ok(datagram) = connection.read_datagram().await {
            if !AudioPacket::is_audio(&datagram) {
                continue;
            }
            if let Err(mpsc::error::TrySendError::Closed(_)) =
                out.try_send((tag, datagram.to_vec()))
            {
                return;
            }
        }
    }))
}
