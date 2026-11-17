//! Platform codec inventory, user switch, license and maintenance operations.
#[cfg(windows)]
use super::mf_common;
#[cfg(any(windows, target_os = "linux"))]
use super::{openh264_codec, openh264_install, openh264_pins};
use super::{Arc, DecoderFactory, EncoderFactory, Factories, OpenH264Report, Path};

/// The encoders and decoders of this Windows host in the rank of plan
/// 2026-11-14 „Bildkette“ 4.10a: NVENC, AMF (BK12, found at run time),
/// then the Media Foundation transforms (hardware MFTs, software MFT);
/// the decoders of FFmpeg (plan 2026-11-17 P8) before those of Media
/// Foundation, both behind „Dekodierer-Bibliothek“; none elsewhere.
#[cfg(windows)]
pub(super) fn media_foundation_codecs() -> Factories {
    let (encoders, decoders) = mf_common::codecs();
    (
        super::vendor_codecs::vendor_encoders(encoders),
        super::ffmpeg_codecs::decoders(decoders),
    )
}

#[cfg(not(windows))]
pub(super) fn media_foundation_codecs() -> Factories {
    (Vec::new(), Vec::new())
}

/// The GPU vendor coders of a Linux host and viewer (BK4: NVENC, VA-API),
/// found at run time; none elsewhere (Windows lists NVENC, AMF and the
/// Media Foundation transforms in `media_foundation_codecs`, Android has
/// its coders in the app).
#[cfg(target_os = "linux")]
pub(super) fn hardware_codecs() -> Factories {
    super::linux_hw_codecs::codecs()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn hardware_codecs() -> Factories {
    (Vec::new(), Vec::new())
}

/// The software AV1 coders of a Linux host and viewer (BK6, B-25:
/// SVT-AV1, dav1d) where the system has them; none elsewhere.
#[cfg(target_os = "linux")]
pub(super) fn software_av1_codecs() -> Factories {
    super::linux_hw_codecs::software_codecs()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn software_av1_codecs() -> Factories {
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
pub enum OpenH264Action {
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
pub fn openh264_license() -> Option<(&'static str, &'static str)> {
    openh264_pins::PIN.map(|_| {
        (
            openh264_pins::CISCO_BINARY_LICENSE,
            openh264_pins::CISCO_BINARY_LICENSE_SHA256,
        )
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn openh264_license() -> Option<(&'static str, &'static str)> {
    None
}

/// Runs `action` on this user's OpenH264.
#[cfg(any(windows, target_os = "linux"))]
pub fn openh264_action(action: &OpenH264Action) -> std::io::Result<()> {
    let store = openh264_install::OpenH264Store::for_user();
    match action {
        OpenH264Action::Enable { accepted } => store.enable(accepted),
        OpenH264Action::Disable => store.disable(),
        OpenH264Action::Remove => store.remove(),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn openh264_action(_action: &OpenH264Action) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "OpenH264 is not offered for this platform",
    ))
}
