//! What each VA-API render node of this system can do for the screen
//! (BK4, B-24, B-38): H.264 encoding (profile, entry point, packed
//! headers, largest picture), H.264 decoding and the video processor
//! (B-18). Probed once per process as the account of that process (the
//! host process runs as the session account, `host_worker_run.rs`), so the
//! answer covers exactly the render nodes this account may open; a node
//! that cannot be opened or initialized is left out without a message.
use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use super::vaapi_ffi::{
    render_nodes, VaApi, VaConfigAttrib, VaDisplay, VA_ATTRIB_NOT_SUPPORTED,
    VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS, VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT,
    VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH, VA_CONFIG_ATTRIB_RATE_CONTROL, VA_CONFIG_ATTRIB_RT_FORMAT,
    VA_ENC_PACKED_HEADER_PICTURE, VA_ENC_PACKED_HEADER_SEQUENCE, VA_ENTRYPOINT_ENC_SLICE,
    VA_ENTRYPOINT_ENC_SLICE_LP, VA_ENTRYPOINT_VIDEO_PROC, VA_ENTRYPOINT_VLD,
    VA_PROFILE_H264_CONSTRAINED_BASELINE, VA_PROFILE_H264_HIGH, VA_PROFILE_H264_MAIN,
    VA_PROFILE_NONE, VA_RC_CBR, VA_RT_FORMAT_YUV420,
};
use super::vaapi_video_probe::{video_decode_caps, video_encode_caps};
pub(super) use super::vaapi_video_probe::{VaVideoDecodeCaps, VaVideoEncodeCaps};
use crate::screen::core::negotiate::H264Profile;
use crate::screen::core::pixel_convert::FrameSize;

/// Directory of the DRM device nodes.
pub(super) const DRI_DIR: &str = "/dev/dri";

/// One way to encode H.264 on a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VaEncodeCaps {
    pub profile: H264Profile,
    pub va_profile: c_int,
    /// `VAEntrypointEncSliceLP` where the driver offers it with CBR, else
    /// `VAEntrypointEncSlice` (B-24).
    pub entrypoint: c_int,
    /// `VA_ENC_PACKED_HEADER_SEQUENCE | _PICTURE` when the driver takes
    /// both, else 0 (the driver writes SPS and PPS itself).
    pub packed_headers: u32,
    pub max: FrameSize,
}

/// H.264 decoding of a device: the VA profile it opens and what it decodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VaDecodeCaps {
    pub va_profile: c_int,
    pub profiles: Vec<H264Profile>,
    pub max: FrameSize,
}

/// One render node and its capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VaDevice {
    pub node: PathBuf,
    pub encode: Vec<VaEncodeCaps>,
    pub decode: Option<VaDecodeCaps>,
    pub video_proc: bool,
    /// HEVC Main and AV1 Main encoding (BK4b, `vaapi_video_probe.rs`).
    pub video_encode: Vec<VaVideoEncodeCaps>,
    /// HEVC Main and AV1 Main decoding (BK4b).
    pub video_decode: Vec<VaVideoDecodeCaps>,
}

impl VaDevice {
    /// The node's file name (`renderD128`), part of the codec names.
    pub(super) fn node_name(&self) -> String {
        self.node
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }
}

/// The devices of this process, probed on first use.
pub(super) fn devices() -> Vec<Arc<VaDevice>> {
    static DEVICES: OnceLock<Vec<Arc<VaDevice>>> = OnceLock::new();
    DEVICES
        .get_or_init(|| {
            let Ok(api) = super::vaapi_ffi::load() else {
                return Vec::new();
            };
            probe_all(&api, Path::new(DRI_DIR))
                .into_iter()
                .map(Arc::new)
                .collect()
        })
        .clone()
}

/// Every node under `dir` that `api` opens with at least one H.264 coder.
pub(super) fn probe_all(api: &Arc<VaApi>, dir: &Path) -> Vec<VaDevice> {
    render_nodes(dir)
        .iter()
        .filter_map(|node| probe(api, node))
        .filter(|device| {
            !device.encode.is_empty()
                || device.decode.is_some()
                || !device.video_encode.is_empty()
                || !device.video_decode.is_empty()
        })
        .collect()
}

fn probe(api: &Arc<VaApi>, node: &Path) -> Option<VaDevice> {
    let display = VaDisplay::open(api.clone(), node).ok()?;
    let profiles = query_profiles(&display);
    let mut encode = Vec::new();
    for (profile, va_profile) in [
        (H264Profile::High, VA_PROFILE_H264_HIGH),
        (
            H264Profile::ConstrainedBaseline,
            VA_PROFILE_H264_CONSTRAINED_BASELINE,
        ),
    ] {
        if !profiles.contains(&va_profile) {
            continue;
        }
        let entrypoints = query_entrypoints(&display, va_profile);
        if let Some(caps) = encode_caps(&display, profile, va_profile, &entrypoints) {
            encode.push(caps);
        }
    }
    let decode = decode_caps(&display, &profiles);
    let video_proc = profiles.contains(&VA_PROFILE_NONE)
        && query_entrypoints(&display, VA_PROFILE_NONE).contains(&VA_ENTRYPOINT_VIDEO_PROC);
    Some(VaDevice {
        node: node.to_path_buf(),
        encode,
        decode,
        video_proc,
        video_encode: video_encode_caps(&display, &profiles),
        video_decode: video_decode_caps(&display, &profiles),
    })
}

fn query_profiles(display: &VaDisplay) -> Vec<c_int> {
    // SAFETY: valid display.
    let most = unsafe { (display.api.max_num_profiles)(display.handle) }.max(0);
    let mut list = vec![0 as c_int; most as usize];
    let mut count: c_int = 0;
    // SAFETY: `list` has room for `vaMaxNumProfiles` entries (va.h).
    let status = unsafe {
        (display.api.query_config_profiles)(display.handle, list.as_mut_ptr(), &mut count)
    };
    if display.check(status, "vaQueryConfigProfiles").is_err() {
        return Vec::new();
    }
    list.truncate(count.clamp(0, most) as usize);
    list
}

pub(super) fn query_entrypoints(display: &VaDisplay, profile: c_int) -> Vec<c_int> {
    // SAFETY: valid display.
    let most = unsafe { (display.api.max_num_entrypoints)(display.handle) }.max(0);
    let mut list = vec![0 as c_int; most as usize];
    let mut count: c_int = 0;
    // SAFETY: `list` has room for `vaMaxNumEntrypoints` entries (va.h).
    let status = unsafe {
        (display.api.query_config_entrypoints)(
            display.handle,
            profile,
            list.as_mut_ptr(),
            &mut count,
        )
    };
    if display.check(status, "vaQueryConfigEntrypoints").is_err() {
        return Vec::new();
    }
    list.truncate(count.clamp(0, most) as usize);
    list
}

/// The values of `kinds` for `profile` and `entrypoint`; a missing
/// attribute reads `VA_ATTRIB_NOT_SUPPORTED`.
pub(super) fn attributes(
    display: &VaDisplay,
    profile: c_int,
    entrypoint: c_int,
    kinds: &[c_int],
) -> Vec<u32> {
    let mut list: Vec<VaConfigAttrib> = kinds
        .iter()
        .map(|kind| VaConfigAttrib {
            kind: *kind,
            value: 0,
        })
        .collect();
    // SAFETY: `list` holds `kinds.len()` attributes for the driver to fill.
    let status = unsafe {
        (display.api.get_config_attributes)(
            display.handle,
            profile,
            entrypoint,
            list.as_mut_ptr(),
            list.len() as c_int,
        )
    };
    if display.check(status, "vaGetConfigAttributes").is_err() {
        return vec![VA_ATTRIB_NOT_SUPPORTED; kinds.len()];
    }
    list.into_iter().map(|attribute| attribute.value).collect()
}

/// The largest picture from the two attribute values, within the protocol
/// limit; an unsupported attribute leaves the protocol limit.
pub(super) fn max_picture(width: u32, height: u32) -> FrameSize {
    let side = |value: u32, limit: u32| {
        if value == VA_ATTRIB_NOT_SUPPORTED || value == 0 {
            limit
        } else {
            value.min(limit)
        }
    };
    FrameSize::new(
        side(width, FrameSize::MAX.width),
        side(height, FrameSize::MAX.height),
    )
}

/// Chooses the entry point for one profile (B-24: low power before normal)
/// from the attributes `(rt_format, rate_control, packed, width, height)`
/// each entry point reports.
pub(super) fn choose_encode(
    profile: H264Profile,
    va_profile: c_int,
    candidates: &[(c_int, [u32; 5])],
) -> Option<VaEncodeCaps> {
    let usable = |values: &[u32; 5]| {
        values[0] != VA_ATTRIB_NOT_SUPPORTED
            && values[0] & VA_RT_FORMAT_YUV420 != 0
            && values[1] != VA_ATTRIB_NOT_SUPPORTED
            && values[1] & VA_RC_CBR != 0
    };
    [VA_ENTRYPOINT_ENC_SLICE_LP, VA_ENTRYPOINT_ENC_SLICE]
        .iter()
        .find_map(|wanted| {
            let (entrypoint, values) = candidates.iter().find(|(e, _)| e == wanted)?;
            usable(values).then(|| {
                let both = VA_ENC_PACKED_HEADER_SEQUENCE | VA_ENC_PACKED_HEADER_PICTURE;
                let packed = values[2];
                VaEncodeCaps {
                    profile,
                    va_profile,
                    entrypoint: *entrypoint,
                    packed_headers: if packed != VA_ATTRIB_NOT_SUPPORTED && packed & both == both {
                        both
                    } else {
                        0
                    },
                    max: max_picture(values[3], values[4]),
                }
            })
        })
}

fn encode_caps(
    display: &VaDisplay,
    profile: H264Profile,
    va_profile: c_int,
    entrypoints: &[c_int],
) -> Option<VaEncodeCaps> {
    let kinds = [
        VA_CONFIG_ATTRIB_RT_FORMAT,
        VA_CONFIG_ATTRIB_RATE_CONTROL,
        VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
        VA_CONFIG_ATTRIB_MAX_PICTURE_WIDTH,
        VA_CONFIG_ATTRIB_MAX_PICTURE_HEIGHT,
    ];
    let candidates: Vec<(c_int, [u32; 5])> = [VA_ENTRYPOINT_ENC_SLICE_LP, VA_ENTRYPOINT_ENC_SLICE]
        .into_iter()
        .filter(|entrypoint| entrypoints.contains(entrypoint))
        .map(|entrypoint| {
            let values = attributes(display, va_profile, entrypoint, &kinds);
            let mut fixed = [VA_ATTRIB_NOT_SUPPORTED; 5];
            fixed.copy_from_slice(&values[..5]);
            (entrypoint, fixed)
        })
        .collect();
    choose_encode(profile, va_profile, &candidates)
}

/// The decode profile: High decodes High and Constrained Baseline streams,
/// Main only Constrained Baseline (H.264 A.2: a CB stream conforms to Main).
pub(super) fn decode_profiles(supported: &[c_int]) -> Option<(c_int, Vec<H264Profile>)> {
    if supported.contains(&VA_PROFILE_H264_HIGH) {
        Some((
            VA_PROFILE_H264_HIGH,
            vec![H264Profile::High, H264Profile::ConstrainedBaseline],
        ))
    } else if supported.contains(&VA_PROFILE_H264_MAIN) {
        Some((VA_PROFILE_H264_MAIN, vec![H264Profile::ConstrainedBaseline]))
    } else if supported.contains(&VA_PROFILE_H264_CONSTRAINED_BASELINE) {
        Some((
            VA_PROFILE_H264_CONSTRAINED_BASELINE,
            vec![H264Profile::ConstrainedBaseline],
        ))
    } else {
        None
    }
}

fn decode_caps(display: &VaDisplay, profiles: &[c_int]) -> Option<VaDecodeCaps> {
    let with_vld: Vec<c_int> = [
        VA_PROFILE_H264_HIGH,
        VA_PROFILE_H264_MAIN,
        VA_PROFILE_H264_CONSTRAINED_BASELINE,
    ]
    .into_iter()
    .filter(|profile| {
        profiles.contains(profile)
            && query_entrypoints(display, *profile).contains(&VA_ENTRYPOINT_VLD)
    })
    .collect();
    let (va_profile, decodes) = decode_profiles(&with_vld)?;
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
    if values[0] != VA_ATTRIB_NOT_SUPPORTED && values[0] & VA_RT_FORMAT_YUV420 == 0 {
        return None;
    }
    Some(VaDecodeCaps {
        va_profile,
        profiles: decodes,
        max: max_picture(values[1], values[2]),
    })
}
