//! The host's sound where this build neither captures nor plays it
//! (Android; plan 2026-11-17 P9 „Audio“): the host never answers a sound
//! offer, and a delivered packet finds no output and is dropped.
use std::io;
use std::process::Command;

pub const CAPTURE_AVAILABLE: bool = false;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "sound capture and playback are unavailable on this platform",
    )
}

pub struct Capture;

impl Capture {
    pub fn open() -> io::Result<Self> {
        Err(unsupported())
    }

    pub fn format(&self) -> (u32, u16) {
        (
            crate::screen::core::audio_wire::SAMPLE_RATE,
            crate::screen::core::audio_wire::CHANNELS,
        )
    }

    pub fn read(&mut self, _out: &mut Vec<f32>) -> io::Result<()> {
        Err(unsupported())
    }
}

pub struct Output;

impl Output {
    pub fn open() -> io::Result<Self> {
        Err(unsupported())
    }

    pub fn wait_space(&mut self) -> io::Result<usize> {
        Err(unsupported())
    }

    pub fn write(&mut self, _samples: &[f32]) -> io::Result<()> {
        Err(unsupported())
    }
}

pub fn prepare_helper(_command: &mut Command) {}
