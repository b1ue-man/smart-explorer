//! The enumeration of the Media Foundation transforms (moved out of
//! `mf_common.rs` by plan 2026-11-14 „Bildkette“ BK6): `MFTEnumEx` per kind
//! and codec, their friendly names, and the activation of one entry.
//!
//! BK6 (B-23): besides H.264 the hardware encoders and the decoders of
//! HEVC (`MFVideoFormat_HEVC`) and AV1 (`MFVideoFormat_AV1`) are listed.
//! Windows has them through the codec extensions of the Store ("HEVC Video
//! Extensions", "AV1 Video Extension") or the GPU vendor's MFTs; a system
//! without them lists none and nothing changes. Encoders of HEVC and AV1
//! are only hardware MFTs (B-23). Sources: `MFTEnumEx`, `MFT_ENUM_FLAG`,
//! `MFT_REGISTER_TYPE_INFO` and the subtype GUIDs as the crate `windows`
//! 0.58.0 binds them (`Win32::Media::MediaFoundation`, read on
//! 2026-11-14).
//!
//! Merge W1 (plan 4.10a, hybrid laptops, BK2a/BK12 E2): hardware encoders
//! are listed per adapter (`MFTEnum2` with `MFT_ENUM_ADAPTER_LUID`), the
//! adapters with a desktop output first, so the MFT of the adapter a
//! Desktop Duplication captures on (Intel Quick Sync on the iGPU of a
//! laptop) stands before the MFT of another adapter; each entry keeps its
//! adapter for the texture route (`mf_d3d_input::attach`) and for the open
//! (`mf_common`: the capture's adapter first).
use std::io;
use std::sync::OnceLock;

use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    eAVEncH264VProfile_Base, eAVEncH264VProfile_ConstrainedBase, eAVEncH264VProfile_High,
    IMFActivate, IMFAttributes, IMFTransform, MFMediaType_Video, MFT_FRIENDLY_NAME_Attribute,
    MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12,
    MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_ADAPTER_LUID, MFT_ENUM_FLAG,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_REGISTER_TYPE_INFO, MF_MT_FRAME_SIZE, MF_MT_MPEG2_LEVEL, MF_MT_MPEG2_PROFILE,
    MF_MT_SUBTYPE,
};
use windows::Win32::System::Com::CoTaskMemFree;

use super::{ensure_com, platform};
use crate::screen::core::negotiate::{CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::dxgi_adapters;

/// The three kinds of transform this strand uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    HardwareEncoder,
    SoftwareEncoder,
    Decoder,
}

/// The compressed subtype of `codec`; `None` for the tiles.
pub(crate) fn subtype(codec: CodecKind) -> Option<GUID> {
    match codec {
        CodecKind::H264 => Some(MFVideoFormat_H264),
        CodecKind::Hevc => Some(MFVideoFormat_HEVC),
        CodecKind::Av1 => Some(MFVideoFormat_AV1),
        CodecKind::Tiles => None,
    }
}

impl Kind {
    fn query(self, codec: CodecKind) -> Option<(GUID, MFT_ENUM_FLAG, GUID, GUID)> {
        let sorted = MFT_ENUM_FLAG_SORTANDFILTER;
        let compressed = subtype(codec)?;
        Some(match self {
            Self::HardwareEncoder => (
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_HARDWARE | sorted,
                MFVideoFormat_NV12,
                compressed,
            ),
            // B-23: HEVC and AV1 only from hardware MFTs.
            Self::SoftwareEncoder if codec != CodecKind::H264 => return None,
            Self::SoftwareEncoder => (
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_SYNCMFT | sorted,
                MFVideoFormat_NV12,
                compressed,
            ),
            Self::Decoder => (
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_SYNCMFT | sorted,
                compressed,
                MFVideoFormat_NV12,
            ),
        })
    }
}

/// A video type of `subtype` for the enumeration.
fn info(subtype: GUID) -> MFT_REGISTER_TYPE_INFO {
    MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype,
    }
}

/// The registered transforms of `kind` for `codec` in the order of their
/// merit.
fn enumerate(kind: Kind, codec: CodecKind) -> io::Result<Vec<IMFActivate>> {
    let Some((category, flags, input, output)) = kind.query(codec) else {
        return Ok(Vec::new());
    };
    let platform = platform()?;
    ensure_com();
    let (input, output) = (info(input), info(output));
    // SAFETY: both type infos outlive the call; `listed` frees the array.
    listed(|array, count| unsafe {
        (platform.enum_ex)(category, flags.0 as u32, &input, &output, array, count)
    })
}

/// The array an enumeration call writes, taken over: each reference is
/// taken out once (released exactly once), the array freed.
fn listed(
    call: impl FnOnce(*mut *mut Option<IMFActivate>, *mut u32) -> windows::core::HRESULT,
) -> io::Result<Vec<IMFActivate>> {
    let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    call(&mut array, &mut count)
        .ok()
        .map_err(io::Error::other)?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: the call returned `count` entries at `array`.
    let entries = unsafe { std::slice::from_raw_parts_mut(array, count as usize) };
    let found = entries.iter_mut().filter_map(Option::take).collect();
    // SAFETY: the enumeration allocated the array with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(array.cast_const().cast())) };
    Ok(found)
}

/// `MFT_ENUM_ADAPTER_LUID` is a `LUID` blob (Microsoft Learn
/// „MFT_ENUM_ADAPTER_LUID attribute“: data type LUID; Chromium
/// `mf_video_encoder_util.cc` writes and reads it with `SetBlob`/`GetBlob`):
/// `LowPart` then `HighPart`, packed as `dxgi_adapters::packed` packs it.
pub(crate) fn luid_bytes(packed: u64) -> [u8; 8] {
    let (low, high) = (packed as u32, (packed >> 32) as u32);
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&low.to_ne_bytes());
    bytes[4..].copy_from_slice(&high.to_ne_bytes());
    bytes
}

/// The packed LUID of an `MFT_ENUM_ADAPTER_LUID` blob.
pub(crate) fn luid_of(bytes: [u8; 8]) -> u64 {
    let low = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let high = u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    (u64::from(high) << 32) | u64::from(low)
}

/// The adapter an MFT or its activation names (`MFT_ENUM_ADAPTER_LUID`).
pub(crate) fn adapter_of(attributes: &IMFAttributes) -> Option<u64> {
    let mut bytes = [0u8; 8];
    let mut size = 0u32;
    let written: *mut u32 = &mut size;
    // SAFETY: an 8-byte buffer for an 8-byte blob; a larger one fails.
    unsafe { attributes.GetBlob(&MFT_ENUM_ADAPTER_LUID, &mut bytes, Some(written)) }.ok()?;
    (size == 8).then(|| luid_of(bytes))
}

/// The hardware encoders of `codec` per adapter (plan 4.10a, merge W1):
/// `MFTEnum2` with `MFT_ENUM_ADAPTER_LUID` for every hardware adapter, an
/// adapter with a desktop output first (Chromium
/// `EnumerateHardwareEncoders` does the same per adapter; Microsoft Learn
/// „MFTEnum2“: the attribute selects the adapter and needs
/// `MFT_ENUM_FLAG_HARDWARE`). Without `MFTEnum2` (before Windows 10 1703)
/// or when it lists nothing, `MFTEnumEx` as before, the adapter known only
/// on a system with one.
fn hardware(codec: CodecKind) -> io::Result<Vec<(IMFActivate, Option<u64>)>> {
    let Some((category, flags, input, output)) = Kind::HardwareEncoder.query(codec) else {
        return Ok(Vec::new());
    };
    let platform = platform()?;
    ensure_com();
    let (input, output) = (info(input), info(output));
    let adapters = dxgi_adapters::adapters();
    let mut found = Vec::new();
    if let (Some(enum2), Some(create)) = (platform.enum2, platform.create_attributes) {
        for adapter in &adapters {
            let luid = dxgi_adapters::packed(adapter.luid);
            let mut raw = std::ptr::null_mut();
            // SAFETY: MFCreateAttributes writes one new reference to `raw`.
            if unsafe { create(&mut raw, 1) }.is_err() || raw.is_null() {
                continue;
            }
            // SAFETY: the reference written above is owned from here on.
            let attributes = unsafe { IMFAttributes::from_raw(raw) };
            // SAFETY: a blob of 8 bytes the store copies.
            if unsafe { attributes.SetBlob(&MFT_ENUM_ADAPTER_LUID, &luid_bytes(luid)) }.is_err() {
                continue;
            }
            // SAFETY: type infos and attributes outlive the call.
            let list = listed(|array, count| unsafe {
                let flags = flags.0 as u32;
                (enum2)(
                    category,
                    flags,
                    &input,
                    &output,
                    attributes.as_raw(),
                    array,
                    count,
                )
            });
            for activate in list.unwrap_or_default() {
                // As Chromium: the activation carries its adapter.
                // SAFETY: an attribute write on an owned activation.
                let _ = unsafe { activate.SetBlob(&MFT_ENUM_ADAPTER_LUID, &luid_bytes(luid)) };
                found.push((activate, Some(luid)));
            }
        }
    }
    if found.is_empty() {
        let single = (adapters.len() == 1).then(|| dxgi_adapters::packed(adapters[0].luid));
        for activate in enumerate(Kind::HardwareEncoder, codec)? {
            let adapter = adapter_of(&activate).or(single);
            found.push((activate, adapter));
        }
    }
    Ok(found)
}

fn friendly_name(activate: &IMFActivate) -> String {
    let mut value = PWSTR::null();
    let mut length = 0u32;
    // SAFETY: the string comes from CoTaskMemAlloc and is freed below.
    let read = unsafe {
        activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut value, &mut length)
    };
    if read.is_err() || value.is_null() {
        return "Media Foundation".into();
    }
    // SAFETY: `value` is a terminated wide string of `length` characters.
    let name = unsafe { value.to_string() }.unwrap_or_else(|_| "Media Foundation".into());
    // SAFETY: allocated by GetAllocatedString.
    unsafe { CoTaskMemFree(Some(value.0.cast_const().cast())) };
    name
}

/// The H.264 transform `index` of `kind`, activated, when it still is `name`.
pub(crate) fn activate(kind: Kind, index: usize, name: &str) -> io::Result<IMFTransform> {
    activate_codec(kind, CodecKind::H264, index, name)
}

/// The transform `index` of `kind` for `codec`, activated, when it still is
/// `name` (hardware encoders in the per-adapter order of `hardware`).
pub(crate) fn activate_codec(
    kind: Kind,
    codec: CodecKind,
    index: usize,
    name: &str,
) -> io::Result<IMFTransform> {
    let entries: Vec<IMFActivate> = match kind {
        Kind::HardwareEncoder => hardware(codec)?.into_iter().map(|(a, _)| a).collect(),
        _ => enumerate(kind, codec)?,
    };
    let entry = entries
        .get(index)
        .filter(|entry| friendly_name(entry) == name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{name} is gone")))?;
    // SAFETY: activates a registered transform in the multithreaded
    // apartment of this thread.
    unsafe { entry.ActivateObject::<IMFTransform>() }.map_err(io::Error::other)
}

/// A hardware encoder found: its name and the adapter it runs on (packed
/// LUID; `None` when the system does not say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub name: String,
    pub adapter: Option<u64>,
}

/// The names of the transforms found, per kind and codec, once per process.
pub(crate) struct Found {
    pub hardware: Vec<Entry>,
    pub software: Vec<String>,
    pub decoders: Vec<String>,
    /// HEVC and AV1 (BK6): hardware encoders and decoders per codec.
    pub video_encoders: Vec<(CodecKind, Vec<Entry>)>,
    pub video_decoders: Vec<(CodecKind, Vec<String>)>,
}

impl Found {
    /// The hardware encoders of `codec`.
    pub(crate) fn encoders(&self, codec: CodecKind) -> &[Entry] {
        if codec == CodecKind::H264 {
            return &self.hardware;
        }
        self.video_encoders
            .iter()
            .find(|(kind, _)| *kind == codec)
            .map_or(&[], |(_, list)| list.as_slice())
    }
}

fn names(kind: Kind, codec: CodecKind) -> Vec<String> {
    enumerate(kind, codec)
        .map(|entries| entries.iter().map(friendly_name).collect())
        .unwrap_or_default()
}

fn hardware_entries(codec: CodecKind) -> Vec<Entry> {
    hardware(codec)
        .map(|list| {
            list.iter()
                .map(|(activate, adapter)| Entry {
                    name: friendly_name(activate),
                    adapter: *adapter,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn found() -> &'static Found {
    static FOUND: OnceLock<Found> = OnceLock::new();
    FOUND.get_or_init(|| {
        let newer = [CodecKind::Hevc, CodecKind::Av1];
        Found {
            hardware: hardware_entries(CodecKind::H264),
            software: names(Kind::SoftwareEncoder, CodecKind::H264),
            decoders: names(Kind::Decoder, CodecKind::H264),
            video_encoders: newer
                .iter()
                .map(|codec| (*codec, hardware_entries(*codec)))
                .collect(),
            video_decoders: newer
                .iter()
                .map(|codec| (*codec, names(Kind::Decoder, *codec)))
                .collect(),
        }
    })
}

/// Media Foundation runs here and has an H.264 encoder (task tests).
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn available() -> bool {
    let found = found();
    !found.hardware.is_empty() || !found.software.is_empty()
}

/// Whether a hardware MFT of `codec` runs on the adapter `luid` (plan
/// 2026-11-17 P3 (b): a vendor encoder of another adapter then gives way).
#[allow(dead_code)] // no longer decides the vendor order (2026-11-17)
pub(crate) fn hardware_mft_on(codec: CodecKind, luid: u64) -> bool {
    found()
        .encoders(codec)
        .iter()
        .any(|entry| entry.adapter == Some(luid))
}

/// Whether a hardware MFT on `own` gives way to one on the capture's
/// adapter `capture` among `entries` (pure, for the task tests): only when
/// its adapter is known, differs, and `entries` hold an MFT of `capture`.
pub(crate) fn prefer_capture_adapter(own: Option<u64>, capture: u64, entries: &[Entry]) -> bool {
    own.is_some_and(|own| own != capture)
        && entries.iter().any(|entry| entry.adapter == Some(capture))
}

/// Only a complete advertised H.264 type is evidence for an extended dimension.
/// Partial/absent types leave the historical offer intact; SetType at open remains decisive.
pub(crate) fn confirmed_offers(
    kind: Kind,
    index: usize,
    name: &str,
    hardware: bool,
) -> Vec<CodecOffer> {
    let Ok(transform) = activate(kind, index, name) else {
        return Vec::new();
    };
    let mut offers = Vec::new();
    for index in 0..64 {
        // SAFETY: enumerating types on a newly activated transform, without streaming.
        let media = unsafe {
            if kind == Kind::Decoder {
                transform.GetInputAvailableType(0, index)
            } else {
                transform.GetOutputAvailableType(0, index)
            }
        };
        let Ok(media) = media else {
            break;
        };
        if unsafe { media.GetGUID(&MF_MT_SUBTYPE) }.ok() != Some(MFVideoFormat_H264) {
            continue;
        }
        let read = || -> windows::core::Result<(u32, u32, u64)> {
            unsafe {
                Ok((
                    media.GetUINT32(&MF_MT_MPEG2_PROFILE)?,
                    media.GetUINT32(&MF_MT_MPEG2_LEVEL)?,
                    media.GetUINT64(&MF_MT_FRAME_SIZE)?,
                ))
            }
        };
        let Ok((profile, level, packed)) = read() else {
            continue;
        };
        let profile = if profile == eAVEncH264VProfile_High.0 as u32 {
            H264Profile::High
        } else if profile == eAVEncH264VProfile_Base.0 as u32
            || profile == eAVEncH264VProfile_ConstrainedBase.0 as u32
        {
            H264Profile::ConstrainedBaseline
        } else {
            continue;
        };
        let Some(level) = crate::screen::core::frame_size_caps::avc_limits(level) else {
            continue;
        };
        let size = FrameSize::new((packed >> 32) as u32, packed as u32);
        if !level.supports(size, 0) {
            continue;
        }
        let max = crate::screen::core::frame_budget::FrameLimits::RESOURCE.max_size;
        let mut offer = CodecOffer::h264(profile, hardware).with_max(FrameSize::new(
            size.width.min(max.width),
            size.height.min(max.height),
        ));
        offer.max_macroblocks = Some(level.macroblocks);
        offer.max_macroblocks_per_second = Some(level.macroblocks_per_second);
        if !offers.contains(&offer) {
            offers.push(offer);
        }
    }
    offers
}

#[cfg(test)]
#[path = "bildkette_w1_mf_task_tests.rs"]
mod bildkette_w1_mf_task_tests;
