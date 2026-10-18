//! Platform side of the screen strand (S4). SCR-M2 brought the parts every
//! platform shares: the test-picture backend and the indicator window of the
//! desktop worker. SCR-M3 adds the codec inventory of a process, the host
//! process that captures and encodes per session (its client in the daemon,
//! its main loop, its lifetime per OS: a job object on Windows, the
//! parent-death signal on Linux), Media Foundation on Windows (loaded at run
//! time) and OpenH264 on Windows and Linux (`native_ffi`, not on Android).
//! Capture and input of each platform follow under `windows/`, `linux_os/`
//! and `android/` (SCR-M4 to SCR-M8). The functions below select the
//! platform's parts with `#[cfg(...)]` so the shared modules stay free of it.
use std::path::Path;
use std::sync::Arc;

use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory, OpenH264Report};
use crate::screen::core::host_backend::ScreenHostBackend;

#[path = "shared/codec_inventory.rs"]
pub(crate) mod codec_inventory;
#[path = "shared/host_worker_client.rs"]
pub(crate) mod host_worker_client;
#[path = "shared/host_worker_main.rs"]
pub(crate) mod host_worker_main;
#[path = "shared/indicator.rs"]
pub(crate) mod indicator;
#[path = "shared/synthetic_backend.rs"]
pub(crate) mod synthetic_backend;

#[cfg(target_os = "linux")]
#[path = "linux_os/worker_death.rs"]
mod worker_lifetime;
#[cfg(windows)]
#[path = "windows/worker_job.rs"]
mod worker_lifetime;
/// Elsewhere the host process ends with the end of its input alone.
#[cfg(not(any(windows, target_os = "linux")))]
mod worker_lifetime {
    pub(super) fn prepare(_command: &mut std::process::Command) {}

    pub(super) fn bind(_child: &std::process::Child) -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn arm_worker() -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_codec.rs"]
pub(crate) mod openh264_codec;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_install.rs"]
pub(crate) mod openh264_install;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_pins.rs"]
pub(crate) mod openh264_pins;

#[cfg(windows)]
#[path = "windows/mf_common.rs"]
mod mf_common;
#[cfg(windows)]
#[path = "windows/mf_decoder.rs"]
mod mf_decoder;
#[cfg(windows)]
#[path = "windows/mf_encoder.rs"]
mod mf_encoder;
#[cfg(windows)]
#[path = "windows/mf_encoder_sync.rs"]
mod mf_encoder_sync;
#[cfg(windows)]
#[path = "windows/mf_media.rs"]
mod mf_media;

/// Encoder and decoder factories of one kind of codec.
type Factories = (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>);

/// The backend that captures on this platform (in the host process). The
/// platform backends of SCR-M4 and SCR-M5 replace the test picture here.
fn platform_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(synthetic_backend::SyntheticBackend)
}

/// The backend of the share host: capture and encoding in the host process,
/// everything else answered by the platform backend.
pub(crate) fn host_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(host_worker_client::WorkerBackend::new(platform_backend()))
}

/// The Media Foundation transforms of this host (Windows); none elsewhere.
#[cfg(windows)]
fn media_foundation_codecs() -> Factories {
    mf_common::codecs()
}

#[cfg(not(windows))]
fn media_foundation_codecs() -> Factories {
    (Vec::new(), Vec::new())
}

/// Whether Media Foundation runs here with an H.264 encoder (task tests).
#[cfg(all(windows, test))]
fn media_foundation_present() -> bool {
    mf_common::available()
}

#[cfg(all(not(windows), test))]
fn media_foundation_present() -> bool {
    false
}

/// The OpenH264 coders of the store in `dir` (`None`: this user's) and its
/// state.
#[cfg(any(windows, target_os = "linux"))]
fn openh264_codecs(
    dir: Option<&Path>,
    verify: bool,
) -> (
    Vec<Arc<dyn EncoderFactory>>,
    Vec<Arc<dyn DecoderFactory>>,
    OpenH264Report,
) {
    let store = dir.map_or_else(openh264_install::OpenH264Store::for_user, |dir| {
        openh264_install::OpenH264Store::at(dir.to_path_buf())
    });
    openh264_codec::codecs(store, verify)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn openh264_codecs(
    _dir: Option<&Path>,
    _verify: bool,
) -> (
    Vec<Arc<dyn EncoderFactory>>,
    Vec<Arc<dyn DecoderFactory>>,
    OpenH264Report,
) {
    use crate::screen::core::codec_registry::OpenH264State;
    let reason = "OpenH264 is not offered for this platform".to_string();
    let report = OpenH264Report::new(OpenH264State::Absent, Some(reason));
    (Vec::new(), Vec::new(), report)
}

/// Whether this user's OpenH264 is switched on (the host process drops a
/// running OpenH264 encoder once it is not).
#[cfg(any(windows, target_os = "linux"))]
fn openh264_switched_on() -> bool {
    openh264_install::OpenH264Store::for_user().switched_on()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn openh264_switched_on() -> bool {
    false
}

/// What the user does with OpenH264 (`se screen codec openh264 …`, the
/// codec view).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenH264Action {
    /// Download if needed and switch on; `accepted` is the SHA-256 of the
    /// license text the user was shown and accepted.
    Enable {
        accepted: String,
    },
    Disable,
    Remove,
}

/// The Cisco license of OpenH264 and its SHA-256; `None` where OpenH264 is
/// not offered.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn openh264_license() -> Option<(&'static str, &'static str)> {
    openh264_pins::PIN.map(|_| {
        (
            openh264_pins::CISCO_BINARY_LICENSE,
            openh264_pins::CISCO_BINARY_LICENSE_SHA256,
        )
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn openh264_license() -> Option<(&'static str, &'static str)> {
    None
}

/// Runs `action` on this user's OpenH264.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn openh264_action(action: &OpenH264Action) -> std::io::Result<()> {
    let store = openh264_install::OpenH264Store::for_user();
    match action {
        OpenH264Action::Enable { accepted } => store.enable(accepted),
        OpenH264Action::Disable => store.disable(),
        OpenH264Action::Remove => store.remove(),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn openh264_action(_action: &OpenH264Action) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "OpenH264 is not offered for this platform",
    ))
}
