//! What the AMF encoder reports of itself (child of `amf_encode.rs`; moved
//! out for the 500-line rule, r1): the largest input picture of a codec
//! (`probe`), the settings a component refuses (`refused_settings`) and the
//! bit-rate ceiling of a session (`session_ceiling`, r1: the level the
//! session writes and the encoder's `*MaxBitrate` capability instead of a
//! fixed value, like Media Foundation's `opened_limit`).
//!
//! Capability names (AMF v1.4.35 headers, read on 2026-11-15):
//! `VideoEncoderVCE.h` `AMF_VIDEO_ENCODER_CAP_MAX_BITRATE` = `MaxBitrate`,
//! `VideoEncoderHEVC.h` `AMF_VIDEO_ENCODER_HEVC_CAP_MAX_BITRATE` =
//! `HevcMaxBitrate`, `VideoEncoderAV1.h` `AMF_VIDEO_ENCODER_AV1_CAP_MAX_BITRATE`
//! = `Av1MaxBitrate`, each „amf_int64; Maximum bit rate in bits“.
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

use super::super::amf_abi::{
    vtable, wide, AmfVariant, CapsVtbl, ComponentVtbl, IoCapsVtbl, This, AMF_OK,
};
use super::super::amf_codecs::names;
use super::super::vendor_encode::avc_level_for;
use super::{context_and_encoder, AmfSession, Owned};
use crate::screen::core::negotiate::{CodecKind, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::{avc_level_max_bitrate_bps, MIN_BITRATE_BPS};

/// The capability holding the encoder's highest bit rate for `codec`.
pub(in super::super) fn max_bitrate_cap(codec: CodecKind) -> Option<&'static str> {
    match codec {
        CodecKind::H264 => Some("MaxBitrate"),
        CodecKind::Hevc => Some("HevcMaxBitrate"),
        CodecKind::Av1 => Some("Av1MaxBitrate"),
        CodecKind::Tiles => None,
    }
}

/// The capabilities of the live `component`; `None` without them.
fn caps_of(component: This) -> Option<Owned> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: a live component; one new caps reference.
    let result = unsafe { (vtable::<ComponentVtbl>(component).get_caps)(component, &mut raw) };
    (result == AMF_OK && !raw.is_null()).then_some(Owned(raw))
}

/// The encoder's own highest bit rate of `codec` (bit/s); `None` when the
/// component does not report it.
fn reported_max_bitrate(component: This, codec: CodecKind) -> Option<u64> {
    let caps = caps_of(component)?;
    let name = wide(max_bitrate_cap(codec)?);
    let mut value = AmfVariant::empty();
    // SAFETY: live caps; NUL-terminated name; an integer needs no clearing.
    let result = unsafe {
        (vtable::<CapsVtbl>(caps.0).storage.get_property)(caps.0, name.as_ptr(), &mut value)
    };
    (result == AMF_OK)
        .then(|| value.as_int64())
        .flatten()
        .and_then(|bits| u64::try_from(bits).ok())
}

/// The bit-rate ceiling of a session: MaxBR of the H.264 level it writes
/// (`ProfileLevel` = `avc_level_for`, profile High, `amf_settings.rs`) and
/// the encoder's capability, whichever is lower; HEVC and AV1 leave their
/// level to the encoder, so only the capability counts. Values below the
/// rate control's floor are no ceiling. `None`: neither is known.
pub(in super::super) fn ceiling_of(level: Option<u64>, reported: Option<u64>) -> Option<u64> {
    [level, reported]
        .into_iter()
        .flatten()
        .filter(|limit| *limit >= MIN_BITRATE_BPS)
        .min()
}

/// `ceiling_of` for the session of `component`.
pub(super) fn session_ceiling(
    component: This,
    codec: CodecKind,
    size: FrameSize,
    fps: u32,
) -> Option<u64> {
    let level = (codec == CodecKind::H264)
        .then(|| {
            let level = avc_level_for(size, fps);
            avc_level_max_bitrate_bps(u32::from(level), H264Profile::High)
        })
        .flatten();
    ceiling_of(level, reported_max_bitrate(component, codec))
}

/// The largest AMF input picture of `codec` on `device`; `None` without the
/// runtime, the component or its capabilities (`AMFCaps::GetInputCaps`).
pub(in super::super) fn probe(device: &ID3D11Device, codec: CodecKind) -> Option<FrameSize> {
    let (_context, component) = context_and_encoder(device, names(codec)?).ok()?;
    let caps = caps_of(component.object)?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: live caps; one new input caps reference.
    let result = unsafe { (vtable::<CapsVtbl>(caps.0).get_input_caps)(caps.0, &mut raw) };
    if result != AMF_OK || raw.is_null() {
        return None;
    }
    let input = Owned(raw);
    let (mut low, mut width, mut height) = (0i32, 0i32, 0i32);
    // SAFETY: live input caps; writable ranges.
    unsafe {
        let calls = vtable::<IoCapsVtbl>(input.0);
        (calls.get_width_range)(input.0, &mut low, &mut width);
        (calls.get_height_range)(input.0, &mut low, &mut height);
    }
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    (width > 0 && height > 0).then(|| {
        FrameSize::new(
            width.min(FrameSize::MAX.width),
            height.min(FrameSize::MAX.height),
        )
    })
}

/// The settings the encoder of `codec` on `device` refuses (B-21, for
/// `se screen codec --json`): a session of the smallest picture is opened
/// and closed again.
pub(in super::super) fn refused_settings(device: &ID3D11Device, codec: CodecKind) -> Vec<String> {
    let size = FrameSize::new(256, 144);
    AmfSession::open(device, codec, size, 60)
        .map(|session| {
            session
                .refused
                .iter()
                .map(|name| name.to_string())
                .collect()
        })
        .unwrap_or_default()
}
