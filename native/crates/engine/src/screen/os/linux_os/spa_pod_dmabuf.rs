//! The pods of the DMA-BUF capture (plan 2026-11-14, package BK8b, B-15):
//! the `EnumFormat` offer with DRM format modifiers, the reader of a
//! negotiated `Format` that may carry a modifier, the `Buffers` answer for
//! DMA-BUF memory, the request for damage metadata and its reader.
//!
//! Contract (PipeWire `doc/dox/internals/dma-buf.dox`, tag 1.2.7, read on
//! 2026-11-14): a consumer offers, per format, one `EnumFormat` with the
//! property `SPA_FORMAT_VIDEO_modifier` (flags `MANDATORY | DONT_FIXATE`,
//! a `SPA_CHOICE_Enum` of `Long`, the first value repeated as default) and
//! after all of them the same format without the property as the shared
//! memory fallback. The allocating side (here the compositor, the producer)
//! fixates the modifier by a test allocation and announces it; the consumer
//! reads the `Format` and answers `SPA_PARAM_BUFFERS_dataType` =
//! `1 << SPA_DATA_DmaBuf` when the format carries a modifier, else shared
//! memory. Sunshine does the same (`src/platform/linux/pipewire.cpp`,
//! `build_format_parameter`, `on_param_changed`, read on 2026-11-14).
//!
//! Sources of the numbers (PipeWire 1.2.7 headers, read on 2026-11-14):
//! `spa/param/format.h` (`SPA_FORMAT_VIDEO_modifier` = 0x20002, after
//! `VIDEO_format` = 0x20001), `spa/pod/pod.h` (`SPA_POD_PROP_FLAG_MANDATORY`
//! = 1 << 3, `DONT_FIXATE` = 1 << 4; `spa_pod_prop` = key, flags, value;
//! `SPA_CHOICE_None` 0, `Range` 1, `Step` 2, `Enum` 3, `Flags` 4),
//! `spa/utils/type.h` (`SPA_TYPE_Int` 4, `SPA_TYPE_Long` 5,
//! `SPA_TYPE_OBJECT_ParamBuffers` 0x40004, `ParamMeta` 0x40005),
//! `spa/buffer/buffer.h` (`SPA_DATA_MemPtr` 1, `MemFd` 2, `DmaBuf` 3),
//! `spa/buffer/meta.h` (`SPA_META_VideoDamage` 3: an array of
//! `spa_meta_region`, an invalid entry or the end of the array ends it;
//! `spa_meta_region_is_valid`: width and height not 0), `spa/param/buffers.h`
//! (`SPA_PARAM_BUFFERS_buffers` 1 „number of buffers (Int)“, `_dataType` 6,
//! `SPA_PARAM_META_type` 1, `_size` 2; `buffers` re-read on 2026-11-15 from
//! the PipeWire 1.2.7 header on GitHub).
//! DRM numbers: Linux 6.8 `include/uapi/drm/drm_fourcc.h` (`fourcc_code`,
//! `DRM_FORMAT_XRGB8888` = 'XR24', `DRM_FORMAT_ARGB8888` = 'AR24',
//! `DRM_FORMAT_MOD_LINEAR` = 0; the linear one is probed in `vaapi_dmabuf.rs`).
use super::{
    choice, id_pod, padded, pod, property, read_u32, word, VideoFormat, CHOICE_ENUM, CHOICE_NONE,
    CHOICE_RANGE, KEY_MEDIA_SUBTYPE, KEY_MEDIA_TYPE, KEY_VIDEO_FORMAT, KEY_VIDEO_FRAMERATE,
    KEY_VIDEO_SIZE, MEDIA_SUBTYPE_RAW, MEDIA_TYPE_VIDEO, OBJECT_FORMAT, PARAM_ENUM_FORMAT,
    TYPE_CHOICE, TYPE_FRACTION, TYPE_ID, TYPE_OBJECT, TYPE_RECTANGLE,
};
use crate::screen::core::pixel_convert::FrameSize;

/// `SPA_FORMAT_VIDEO_modifier`.
pub const KEY_VIDEO_MODIFIER: u32 = 0x20002;
const TYPE_INT: u32 = 4;
const TYPE_LONG: u32 = 5;
/// `SPA_POD_PROP_FLAG_MANDATORY` and `SPA_POD_PROP_FLAG_DONT_FIXATE`.
pub const PROP_FLAG_MANDATORY: u32 = 1 << 3;
pub const PROP_FLAG_DONT_FIXATE: u32 = 1 << 4;
const CHOICE_FLAGS: u32 = 4;
/// `SPA_DATA_DmaBuf` of `enum spa_data_type` (MemPtr 1, MemFd 2, DmaBuf 3).
pub const DATA_DMABUF: u32 = 3;
/// `SPA_META_VideoDamage` and the size of one `spa_meta_region`
/// (`spa_point` two `i32`, `spa_rectangle` two `u32`).
pub const META_VIDEO_DAMAGE: u32 = 3;
pub const REGION_BYTES: usize = 16;
/// Regions asked for per buffer (Sunshine asks for 16 as well).
pub const DAMAGE_REGIONS: u32 = 16;
const OBJECT_PARAM_BUFFERS: u32 = 0x40004;
const OBJECT_PARAM_META: u32 = 0x40005;
/// `SPA_PARAM_Buffers` and `SPA_PARAM_Meta` as object ids.
const PARAM_BUFFERS: u32 = 5;
const PARAM_META: u32 = 6;
const KEY_BUFFERS_DATA_TYPE: u32 = 6;
/// `SPA_PARAM_BUFFERS_buffers`.
const KEY_BUFFERS_BUFFERS: u32 = 1;
/// The PipeWire buffers the DMA-BUF path holds at once (W2 hand-off): the
/// newest picture in the `GpuSlot`, the picture waiting in the capture
/// stage of the host process and the one the encoder imports.
pub const DMABUF_HELD: u32 = 3;
/// Least and wished number of buffers (r1): every held one plus the one the
/// compositor renders into, so the stream never waits for a buffer back.
pub const DMABUF_BUFFERS_MIN: u32 = DMABUF_HELD + 1;
/// Most buffers: the upper end of mutter's own range (`meta-screen-cast-
/// stream-src.c` 47.0, `SPA_POD_CHOICE_RANGE_Int (16, 2, 16)`, read on
/// 2026-11-15); xdg-desktop-portal-wlr 0.7.1 offers up to 32.
pub const DMABUF_BUFFERS_MAX: u32 = 16;

pub const DRM_FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");

impl VideoFormat {
    /// The DRM FOURCC of the same memory layout: SPA `BGRx` is the bytes
    /// B, G, R, x, which `drm_fourcc.h` names `XRGB8888` ("[31:0] x:R:G:B
    /// little endian"). Only the two layouts the VA import of this program
    /// knows (`VA_FOURCC_BGRX`, `VA_FOURCC_BGRA`); `RGBx`/`RGBA` stay SHM.
    pub fn drm_fourcc(self) -> Option<u32> {
        match self {
            Self::Bgrx => Some(DRM_FORMAT_XRGB8888),
            Self::Bgra => Some(DRM_FORMAT_ARGB8888),
            Self::Rgbx | Self::Rgba => None,
        }
    }

    pub fn from_drm_fourcc(fourcc: u32) -> Option<Self> {
        [Self::Bgrx, Self::Bgra]
            .into_iter()
            .find(|format| format.drm_fourcc() == Some(fourcc))
    }
}

/// One format the GPU encoder imports and the modifiers it imports it with
/// (preferred first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmaBufOffer {
    pub format: VideoFormat,
    pub modifiers: Vec<u64>,
}

fn property_flags(out: &mut Vec<u8>, key: u32, flags: u32) {
    word(out, key);
    word(out, flags);
}

/// A choice of 8-byte `Long` values.
fn long_choice(out: &mut Vec<u8>, kind: u32, values: &[u64]) {
    let mut body = Vec::new();
    word(&mut body, kind);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, TYPE_LONG);
    for value in values {
        body.extend_from_slice(&value.to_le_bytes());
    }
    pod(out, TYPE_CHOICE, &body);
}

/// The DMA-BUF `EnumFormat` of `offer`: the one format, the modifiers as a
/// mandatory, not to be fixated enum (first repeated as the default),
/// sizes and frame rates as in the shared memory offer. `None` for an offer
/// without modifiers or with a format the import does not know.
pub fn enum_format_dmabuf_pod(offer: &DmaBufOffer) -> Option<Vec<u8>> {
    offer.format.drm_fourcc()?;
    let first = *offer.modifiers.first()?;
    let mut body = Vec::new();
    word(&mut body, OBJECT_FORMAT);
    word(&mut body, PARAM_ENUM_FORMAT);
    property(&mut body, KEY_MEDIA_TYPE);
    id_pod(&mut body, MEDIA_TYPE_VIDEO);
    property(&mut body, KEY_MEDIA_SUBTYPE);
    id_pod(&mut body, MEDIA_SUBTYPE_RAW);
    property(&mut body, KEY_VIDEO_FORMAT);
    id_pod(&mut body, offer.format.id());
    property_flags(
        &mut body,
        KEY_VIDEO_MODIFIER,
        PROP_FLAG_MANDATORY | PROP_FLAG_DONT_FIXATE,
    );
    let mut values = vec![first];
    values.extend_from_slice(&offer.modifiers);
    long_choice(&mut body, CHOICE_ENUM, &values);
    property(&mut body, KEY_VIDEO_SIZE);
    choice(
        &mut body,
        CHOICE_RANGE,
        TYPE_RECTANGLE,
        8,
        &[1920, 1080, 1, 1, 16384, 16384],
    );
    property(&mut body, KEY_VIDEO_FRAMERATE);
    choice(
        &mut body,
        CHOICE_RANGE,
        TYPE_FRACTION,
        8,
        &[0, 1, 0, 1, 1000, 1],
    );
    let mut out = Vec::with_capacity(body.len() + 8);
    pod(&mut out, TYPE_OBJECT, &body);
    Some(out)
}

/// Every `EnumFormat` of a connection: the DMA-BUF ones first (PipeWire
/// prefers the earlier ones), then the shared memory offer, which is the
/// fallback in the same offer. Without offers it is the shared memory one.
pub fn offer_pods(offers: &[DmaBufOffer]) -> Vec<Vec<u8>> {
    let mut pods: Vec<Vec<u8>> = offers.iter().filter_map(enum_format_dmabuf_pod).collect();
    pods.push(super::enum_format_pod());
    pods
}

/// A negotiated `Format`: the picture and, for DMA-BUF, the modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Negotiated {
    pub format: VideoFormat,
    pub size: FrameSize,
    /// Present only when the producer chose DMA-BUF.
    pub modifier: Option<u64>,
    /// The modifier still is an enum with `DONT_FIXATE`: the producer has
    /// not fixated yet and announces the fixated format next.
    pub fixation_pending: bool,
}

/// The first value of a choice or a plain value of `size` bytes and `kind`.
fn first_value(value_type: u32, value: &[u8], kind: u32, size: usize) -> Option<(&[u8], u32)> {
    if value_type == kind {
        return Some((value.get(..size)?, CHOICE_NONE));
    }
    if value_type != TYPE_CHOICE {
        return None;
    }
    let (choice_kind, child_size, child_type) = (
        read_u32(value, 0)?,
        read_u32(value, 8)? as usize,
        read_u32(value, 12)?,
    );
    if child_type != kind || child_size != size {
        return None;
    }
    Some((value.get(16..16 + size)?, choice_kind))
}

/// Reads format, size and modifier from a negotiated `Format` object (plain
/// values or fixated choices; for format and size a choice with
/// alternatives is an offer, no value). `None` when either picture value
/// is missing or unknown.
pub fn parse_negotiated(bytes: &[u8]) -> Option<Negotiated> {
    let size = read_u32(bytes, 0)? as usize;
    if read_u32(bytes, 4)? != TYPE_OBJECT || size < 8 {
        return None;
    }
    let body = bytes.get(8..8usize.checked_add(size)?)?;
    if read_u32(body, 0)? != OBJECT_FORMAT {
        return None;
    }
    let (mut format, mut frame, mut modifier, mut pending) = (None, None, None, false);
    let mut at = 8;
    while at + 16 <= body.len() {
        let key = read_u32(body, at)?;
        let flags = read_u32(body, at + 4)?;
        let value_size = read_u32(body, at + 8)? as usize;
        let value_type = read_u32(body, at + 12)?;
        let value = body.get(at + 16..at.checked_add(16)?.checked_add(value_size)?)?;
        match key {
            KEY_VIDEO_FORMAT => {
                if let Some((id, CHOICE_NONE)) = first_value(value_type, value, TYPE_ID, 4) {
                    format = VideoFormat::from_id(read_u32(id, 0)?);
                }
            }
            KEY_VIDEO_SIZE => {
                if let Some((rect, CHOICE_NONE)) = first_value(value_type, value, TYPE_RECTANGLE, 8)
                {
                    frame = Some(FrameSize::new(read_u32(rect, 0)?, read_u32(rect, 4)?));
                }
            }
            KEY_VIDEO_MODIFIER => {
                if let Some((long, kind)) = first_value(value_type, value, TYPE_LONG, 8) {
                    modifier = Some(u64::from_le_bytes(long.try_into().ok()?));
                    pending = kind != CHOICE_NONE && flags & PROP_FLAG_DONT_FIXATE != 0;
                }
            }
            _ => {}
        }
        at = at.checked_add(16)?.checked_add(padded(value_size))?;
    }
    Some(Negotiated {
        format: format?,
        size: frame?,
        modifier,
        fixation_pending: pending,
    })
}

/// The `Buffers` answer for a DMA-BUF format: the number of buffers as a
/// range (wished and least `DMABUF_BUFFERS_MIN`, most `DMABUF_BUFFERS_MAX`)
/// and the data type (the plane count and sizes come from the producer's
/// allocation; dma-buf.dox "Size of DMA-BUFs": consumers ignore `maxsize`
/// and `chunk.size`).
pub fn dmabuf_buffers_pod() -> Vec<u8> {
    let mut body = Vec::new();
    word(&mut body, OBJECT_PARAM_BUFFERS);
    word(&mut body, PARAM_BUFFERS);
    property(&mut body, KEY_BUFFERS_BUFFERS);
    choice(
        &mut body,
        CHOICE_RANGE,
        TYPE_INT,
        4,
        &[DMABUF_BUFFERS_MIN, DMABUF_BUFFERS_MIN, DMABUF_BUFFERS_MAX],
    );
    property(&mut body, KEY_BUFFERS_DATA_TYPE);
    choice(&mut body, CHOICE_FLAGS, TYPE_INT, 4, &[1 << DATA_DMABUF]);
    let mut out = Vec::new();
    pod(&mut out, TYPE_OBJECT, &body);
    out
}

/// The `Meta` parameter asking for `SPA_META_VideoDamage` with room for
/// one to `DAMAGE_REGIONS` regions (Sunshine `on_param_changed`).
pub fn damage_meta_pod() -> Vec<u8> {
    let most = DAMAGE_REGIONS * REGION_BYTES as u32;
    let mut body = Vec::new();
    word(&mut body, OBJECT_PARAM_META);
    word(&mut body, PARAM_META);
    property(&mut body, 1);
    id_pod(&mut body, META_VIDEO_DAMAGE);
    property(&mut body, 2);
    choice(
        &mut body,
        CHOICE_RANGE,
        TYPE_INT,
        4,
        &[most, REGION_BYTES as u32, most],
    );
    let mut out = Vec::new();
    pod(&mut out, TYPE_OBJECT, &body);
    out
}

/// What a damage block says about its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    /// The first region is invalid: the producer reports no change.
    Nothing,
    /// That many valid regions before the end or the first invalid one.
    Regions(usize),
}

/// Reads a `SPA_META_VideoDamage` block (`spa_meta_region` array).
pub fn parse_damage(block: &[u8]) -> Damage {
    let valid = block
        .chunks_exact(REGION_BYTES)
        .take_while(|region| {
            let width = u32::from_le_bytes([region[8], region[9], region[10], region[11]]);
            let height = u32::from_le_bytes([region[12], region[13], region[14], region[15]]);
            width != 0 && height != 0
        })
        .count();
    if valid == 0 {
        Damage::Nothing
    } else {
        Damage::Regions(valid)
    }
}
