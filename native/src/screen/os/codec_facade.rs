//! Platform codec inventory, user switch, license and maintenance operations.
#[cfg(windows)]
use super::mf_common;
#[cfg(any(windows, target_os = "linux"))]
use super::{openh264_codec, openh264_install, openh264_pins};
use super::{Arc, DecoderFactory, EncoderFactory, Factories, OpenH264Report, Path};

/// The Media Foundation transforms of this host (Windows); none elsewhere.
#[cfg(windows)]
pub(super) fn media_foundation_codecs() -> Factories {
    mf_common::codecs()
}

#[cfg(not(windows))]
pub(super) fn media_foundation_codecs() -> Factories {
    (Vec::new(), Vec::new())
}

/// Whether Media Foundation runs here with an H.264 encoder (task tests).
#[cfg(all(windows, test))]
pub(super) fn media_foundation_present() -> bool {
    mf_common::available()
}

#[cfg(all(not(windows), test))]
pub(super) fn media_foundation_present() -> bool {
    false
}

/// The OpenH264 coders of the store in `dir` (`None`: this user's) and its
/// state.
#[cfg(any(windows, target_os = "linux"))]
pub(super) fn openh264_codecs(
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
pub(super) fn openh264_codecs(
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
pub(super) fn openh264_switched_on() -> bool {
    openh264_install::OpenH264Store::for_user().switched_on()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn openh264_switched_on() -> bool {
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
