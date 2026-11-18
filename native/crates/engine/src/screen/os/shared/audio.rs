//! The host's sound in a screen session (plan 2026-11-17 P9 „Audio“),
//! platform side. Format, jitter buffer and codec are in `core/audio_*`.
//!
//! Host: the daemon answers a viewer's sound offer (`HELLO_OK` `audio`)
//! when this platform can capture its output, and attaches a link to the
//! session's connection (`audio_host.rs`). The viewer's `AUDIO` on starts a
//! capture process in the user's session (`--screen-host-worker` with
//! `SE_SCREEN_AUDIO_HOST`, like the screen host process: on Linux as the
//! followed session's account with its variables, so it reaches that
//! session's PipeWire or PulseAudio), which captures the system output
//! (WASAPI loopback, PulseAudio/PipeWire monitor of the default sink),
//! converts it to 48 kHz stereo, codes 5 ms Opus frames and writes them to
//! its standard output (`audio_helper.rs`). The daemon sends each packet
//! as a QUIC datagram of the session. `AUDIO` off, the end of the session
//! or of the connection ends the capture process.
//!
//! Viewer: the daemon that holds the viewer's connection forwards the
//! datagrams to the viewer window (`RELAY_AUDIO`, `relay`); the window's
//! process plays them (`deliver`, `set_playing`, `audio_viewer.rs`) through
//! a jitter buffer and the default output device (WASAPI shared mode,
//! PulseAudio/PipeWire). Android neither captures nor plays (yet): it never
//! answers an offer, and its viewer never switches the sound on.

#[path = "audio_helper.rs"]
mod helper;
#[path = "audio_host.rs"]
mod host;
#[path = "audio_viewer.rs"]
mod viewer;

#[cfg(windows)]
#[path = "../windows/audio_wasapi.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "../linux_os/audio_pulse.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "audio_none.rs"]
mod platform;

pub use helper::{requested as helper_requested, run as run_helper};
pub use viewer::{deliver, relay, report, set_playing, RelayGuard};

/// The sound link of this host for the session (`ScreenHostBackend::audio`
/// of the host-process backend); `None` where this platform cannot capture
/// its output.
pub fn host() -> Option<std::sync::Arc<dyn crate::screen::core::audio_port::AudioHost>> {
    platform::CAPTURE_AVAILABLE.then(|| {
        std::sync::Arc::new(host::CaptureLink)
            as std::sync::Arc<dyn crate::screen::core::audio_port::AudioHost>
    })
}
