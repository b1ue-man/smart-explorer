//! The capture process of the host's sound (plan 2026-11-17 P9 „Audio“):
//! `--screen-host-worker` with `SE_SCREEN_AUDIO_HOST` set. It captures the
//! system output of its session, converts it to 48 kHz stereo, cuts 5 ms
//! frames, codes them with Opus and writes each packet to its standard
//! output (`audio_wire::write_piped`). It ends when its standard input
//! ends (the daemon stopped the sound or ended), when the output pipe
//! breaks, or when the capture fails; the daemon starts it again while the
//! viewer wants the sound. Its standard input carries the Opus rate as
//! 4-byte big-endian values (review 2026-11-19, Codex 7/7:
//! `audio_codec::opus_bitrate_for` of the viewer's „Höchste Datenrate“).
//! Diagnostics go to standard error only.
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

use crate::screen::core::audio_codec::{AudioEncoder, OPUS_BITRATE};
use crate::screen::core::audio_convert::{FrameAssembler, ToStereo48};
use crate::screen::core::audio_wire::{write_piped, AudioCodec, AudioPacket};

/// The Opus rate the daemon set last (`run`'s input).
static RATE: AtomicI32 = AtomicI32::new(OPUS_BITRATE);

/// Whether this process is the capture process.
pub fn requested() -> bool {
    std::env::var_os(super::host::HELPER_ENV).is_some()
}

/// Runs the capture process; its exit code.
pub fn run() -> i32 {
    // The end of the input is the daemon's stop; before it, Opus rates.
    let watcher = std::thread::Builder::new()
        .name("screen-audio-input".into())
        .spawn(|| {
            let mut rate = [0u8; 4];
            let mut input = io::stdin();
            while input.read_exact(&mut rate).is_ok() {
                RATE.store(i32::from_be_bytes(rate), Ordering::Release);
            }
            std::process::exit(0);
        });
    if let Err(error) = watcher {
        eprintln!("screen audio: {error}");
        return 1;
    }
    match capture() {
        Ok(()) => 0,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(error) => {
            eprintln!("screen audio: {error}");
            1
        }
    }
}

fn capture() -> io::Result<()> {
    let mut source = super::platform::Capture::open()?;
    let (rate, channels) = source.format();
    let mut convert = ToStereo48::new(rate, channels)?;
    let mut assembler = FrameAssembler::default();
    let mut encoder = AudioEncoder::new(AudioCodec::Opus)?;
    let started = Instant::now();
    let mut output = io::stdout().lock();
    let mut raw = Vec::new();
    let mut stereo = Vec::new();
    let mut payload = Vec::new();
    let mut sequence = 0u32;
    loop {
        raw.clear();
        source.read(&mut raw)?;
        // The last sample of what was just read was captured about now.
        let now_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        stereo.clear();
        convert.push(&raw, &mut stereo);
        assembler.push(&stereo, now_us);
        while let Some((frame, host_us)) = assembler.pop() {
            encoder.set_bitrate(RATE.load(Ordering::Acquire));
            encoder.encode(&frame, &mut payload)?;
            let packet = AudioPacket {
                codec: encoder.codec(),
                sequence,
                host_us,
                payload: std::mem::take(&mut payload),
            };
            write_piped(&mut output, &packet.encode()?)?;
            payload = packet.payload;
            sequence = sequence.wrapping_add(1);
        }
        output.flush()?;
    }
}
