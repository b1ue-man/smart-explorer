//! What a VA-API render node can do with HEVC Main and AV1 Main (plan
//! 2026-11-14 „Bildkette“ BK4b, B-24, B-30, B-38), probed next to the
//! H.264 capabilities of `vaapi_probe.rs` with the same rules: low-power
//! entry point before the normal one where it offers CBR (B-24, Sunshine
//! `vaapi.cpp:215`), the largest picture from the driver, nothing reported
//! for a profile the driver lacks. Constants: libva 2.20.0 `va.h`
//! (`VAProfileHEVCMain` 17, `VAProfileAV1Profile0` 32,
//! `VAConfigAttribPredictionDirection` 39, `VAConfigAttribEncHEVCFeatures`
//! 50, `…EncHEVCBlockSizes` 51, `…EncAV1` 52, `…EncAV1Ext1` 53,
//! `…EncAV1Ext2` 54), `va_enc_hevc.h` (`VAConfigAttribValEncHEVCFeatures`:
//! two bits per tool, `scaling_lists` bits 2–3, `separate_colour_planes`
//! bits 0–1).
use std::ffi::c_int;

use super::vaapi_ffi::{
    VaDisplay, VA_ATTRIB_NOT_SUPPORTED, VA_CONFIG_ATTRIB_ENC_AV1, VA_CONFIG_ATTRIB_ENC_AV1_EXT1,
    VA_CONFIG_ATTRIB_ENC_AV1_EXT2, VA_CONFIG_ATTRIB_ENC_HEVC_BLOCK_SIZES,
    VA_CONFIG_ATTRIB_ENC_HEVC_FEATURES, VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
    VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT, VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH,
    VA_CONFIG_ATTRIB_PREDICTION_DIRECTION, VA_CONFIG_ATTRIB_RATE_CONTROL,
    VA_CONFIG_ATTRIB_RT_FORMAT, VA_ENTRYPOINT_ENC_SLICE, VA_ENTRYPOINT_ENC_SLICE_LP,
    VA_ENTRYPOINT_VLD, VA_FEATURE_REQUIRED, VA_PROFILE_AV1_PROFILE0, VA_PROFILE_HEVC_MAIN,
    VA_RC_CBR, VA_RT_FORMAT_YUV420,
};
use super::vaapi_probe::{attributes, max_picture, query_entrypoints};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::pixel_convert::FrameSize;

/// Attribute values of one entry point, in this order.
pub(super) const RT_FORMAT: usize = 0;
pub(super) const RATE_CONTROL: usize = 1;
pub(super) const PACKED: usize = 2;
pub(super) const MAX_WIDTH: usize = 3;
pub(super) const MAX_HEIGHT: usize = 4;
/// `VAConfigAttribEncHEVCFeatures` or `VAConfigAttribEncAV1`.
pub(super) const FEATURES: usize = 5;
/// `VAConfigAttribEncHEVCBlockSizes` or `VAConfigAttribEncAV1Ext1`.
pub(super) const BLOCKS: usize = 6;
/// `VAConfigAttribEncAV1Ext2` (HEVC: not supported).
pub(super) const EXT2: usize = 7;
pub(super) const PREDICTION: usize = 8;
pub(super) const VALUES: usize = 9;

/// One way to encode HEVC Main or AV1 Main (4:2:0, 8 bit) on a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VaVideoEncodeCaps {
    pub codec: CodecKind,
    pub va_profile: c_int,
    /// `VAEntrypointEncSliceLP` where it offers CBR, else `EncSlice`.
    pub entrypoint: c_int,
    /// Every `VA_ENC_PACKED_HEADER_*` bit the driver takes; the session
    /// asks for the ones it supplies (`VA_ATTRIB_NOT_SUPPORTED` reads 0).
    pub packed: u32,
    pub max: FrameSize,
    /// The raw attribute values (`VA_ATTRIB_NOT_SUPPORTED` when absent).
    pub features: u32,
    pub blocks: u32,
    pub ext2: u32,
    pub prediction: u32,
}

/// HEVC Main or AV1 Main decoding of a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VaVideoDecodeCaps {
    pub codec: CodecKind,
    pub va_profile: c_int,
    pub max: FrameSize,
}

/// The codecs of BK4b and their VA profile.
pub(super) const VIDEO_PROFILES: [(CodecKind, c_int); 2] = [
    (CodecKind::Hevc, VA_PROFILE_HEVC_MAIN),
    (CodecKind::Av1, VA_PROFILE_AV1_PROFILE0),
];

/// The attribute kinds queried for `codec`, in the order of the indices.
fn kinds(codec: CodecKind) -> Vec<c_int> {
    let (features, blocks) = match codec {
        CodecKind::Av1 => (VA_CONFIG_ATTRIB_ENC_AV1, VA_CONFIG_ATTRIB_ENC_AV1_EXT1),
        _ => (
            VA_CONFIG_ATTRIB_ENC_HEVC_FEATURES,
            VA_CONFIG_ATTRIB_ENC_HEVC_BLOCK_SIZES,
        ),
    };
    let mut list = vec![
        VA_CONFIG_ATTRIB_RT_FORMAT,
        VA_CONFIG_ATTRIB_RATE_CONTROL,
        VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
        VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH,
        VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT,
        features,
        blocks,
    ];
    if codec == CodecKind::Av1 {
        list.push(VA_CONFIG_ATTRIB_ENC_AV1_EXT2);
    }
    list.push(VA_CONFIG_ATTRIB_PREDICTION_DIRECTION);
    list
}

/// The values of `kinds(codec)` spread over the fixed indices.
fn spread(codec: CodecKind, values: &[u32]) -> [u32; VALUES] {
    let mut out = [VA_ATTRIB_NOT_SUPPORTED; VALUES];
    let mut at = 0;
    for (index, slot) in out.iter_mut().enumerate() {
        if index == EXT2 && codec != CodecKind::Av1 {
            continue;
        }
        if let Some(value) = values.get(at) {
            *slot = *value;
        }
        at += 1;
    }
    out
}

/// Whether a device can take the HEVC stream this program writes: a tool
/// it requires and the program does not supply (scaling lists, separate
/// colour planes) rules the entry point out.
fn hevc_usable(features: u32) -> bool {
    if features == VA_ATTRIB_NOT_SUPPORTED {
        return true;
    }
    let field = |shift: u32| (features >> shift) & 3;
    field(0) != VA_FEATURE_REQUIRED && field(2) != VA_FEATURE_REQUIRED
}

/// Chooses the entry point of `codec` (B-24: low power before normal) from
/// the attribute values each entry point reports.
pub(super) fn choose_video_encode(
    codec: CodecKind,
    va_profile: c_int,
    candidates: &[(c_int, [u32; VALUES])],
) -> Option<VaVideoEncodeCaps> {
    let usable = |values: &[u32; VALUES]| {
        let rt = values[RT_FORMAT];
        let rc = values[RATE_CONTROL];
        rt != VA_ATTRIB_NOT_SUPPORTED
            && rt & VA_RT_FORMAT_YUV420 != 0
            && rc != VA_ATTRIB_NOT_SUPPORTED
            && rc & VA_RC_CBR != 0
            && (codec != CodecKind::Hevc || hevc_usable(values[FEATURES]))
    };
    [VA_ENTRYPOINT_ENC_SLICE_LP, VA_ENTRYPOINT_ENC_SLICE]
        .iter()
        .find_map(|wanted| {
            let (entrypoint, values) = candidates.iter().find(|(e, _)| e == wanted)?;
            usable(values).then(|| VaVideoEncodeCaps {
                codec,
                va_profile,
                entrypoint: *entrypoint,
                packed: match values[PACKED] {
                    VA_ATTRIB_NOT_SUPPORTED => 0,
                    packed => packed,
                },
                max: max_picture(values[MAX_WIDTH], values[MAX_HEIGHT]),
                features: values[FEATURES],
                blocks: values[BLOCKS],
                ext2: values[EXT2],
                prediction: values[PREDICTION],
            })
        })
}

/// HEVC Main and AV1 Main encoding of `display` among `profiles`.
pub(super) fn video_encode_caps(display: &VaDisplay, profiles: &[c_int]) -> Vec<VaVideoEncodeCaps> {
    let mut found = Vec::new();
    for (codec, va_profile) in VIDEO_PROFILES {
        if !profiles.contains(&va_profile) {
            continue;
        }
        let entrypoints = query_entrypoints(display, va_profile);
        let kinds = kinds(codec);
        let candidates: Vec<(c_int, [u32; VALUES])> =
            [VA_ENTRYPOINT_ENC_SLICE_LP, VA_ENTRYPOINT_ENC_SLICE]
                .into_iter()
                .filter(|entrypoint| entrypoints.contains(entrypoint))
                .map(|entrypoint| {
                    let values = attributes(display, va_profile, entrypoint, &kinds);
                    (entrypoint, spread(codec, &values))
                })
                .collect();
        if let Some(caps) = choose_video_encode(codec, va_profile, &candidates) {
            found.push(caps);
        }
    }
    found
}

/// The decode capability of `codec` from the VLD attributes
/// `(rt_format, max width, max height)`; `None` without 4:2:0.
pub(super) fn decode_from(
    codec: CodecKind,
    va_profile: c_int,
    values: [u32; 3],
) -> Option<VaVideoDecodeCaps> {
    if values[0] != VA_ATTRIB_NOT_SUPPORTED && values[0] & VA_RT_FORMAT_YUV420 == 0 {
        return None;
    }
    Some(VaVideoDecodeCaps {
        codec,
        va_profile,
        max: max_picture(values[1], values[2]),
    })
}

/// HEVC Main and AV1 Main decoding of `display` among `profiles`.
pub(super) fn video_decode_caps(display: &VaDisplay, profiles: &[c_int]) -> Vec<VaVideoDecodeCaps> {
    VIDEO_PROFILES
        .into_iter()
        .filter(|(_, va_profile)| {
            profiles.contains(va_profile)
                && query_entrypoints(display, *va_profile).contains(&VA_ENTRYPOINT_VLD)
        })
        .filter_map(|(codec, va_profile)| {
            let values = attributes(
                display,
                va_profile,
                VA_ENTRYPOINT_VLD,
                &[
                    VA_CONFIG_ATTRIB_RT_FORMAT,
                    VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH,
                    VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT,
                ],
            );
            decode_from(codec, va_profile, [values[0], values[1], values[2]])
        })
        .collect()
}
