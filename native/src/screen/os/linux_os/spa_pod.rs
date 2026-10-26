//! The SPA pods PipeWire needs for a screen stream (SCR-M5, U19-5), built
//! by hand: the pod builder of PipeWire is `static inline` in a header and
//! no symbol of `libpipewire-0.3.so.0`. Only what the stream needs exists:
//! the `EnumFormat` object that offers raw video, and the reader of the
//! negotiated `Format` object. All numbers are little-endian (the layout of
//! `spa/pod/pod.h` on every Linux target of this program is its native one;
//! big-endian Linux targets are not supported by the pod writer).
//!
//! A pod is `size: u32` (length of the body without padding), `type: u32`,
//! the body, padded with zero bytes to a multiple of 8.
use crate::screen::core::pixel_convert::FrameSize;

const TYPE_ID: u32 = 3;
const TYPE_RECTANGLE: u32 = 10;
const TYPE_FRACTION: u32 = 11;
const TYPE_OBJECT: u32 = 15;
const TYPE_CHOICE: u32 = 19;
/// `SPA_TYPE_OBJECT_Format`.
const OBJECT_FORMAT: u32 = 0x40003;
/// `SPA_PARAM_EnumFormat` as the object id.
const PARAM_ENUM_FORMAT: u32 = 3;
const CHOICE_NONE: u32 = 0;
const CHOICE_RANGE: u32 = 1;
const CHOICE_ENUM: u32 = 3;
const KEY_MEDIA_TYPE: u32 = 1;
const KEY_MEDIA_SUBTYPE: u32 = 2;
const KEY_VIDEO_FORMAT: u32 = 0x20001;
const KEY_VIDEO_SIZE: u32 = 0x20003;
const KEY_VIDEO_FRAMERATE: u32 = 0x20004;
/// `SPA_MEDIA_TYPE_video` and `SPA_MEDIA_SUBTYPE_raw`.
const MEDIA_TYPE_VIDEO: u32 = 2;
const MEDIA_SUBTYPE_RAW: u32 = 1;

/// The raw video formats the stream reads (`SPA_VIDEO_FORMAT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VideoFormat {
    Rgbx,
    Bgrx,
    Rgba,
    Bgra,
}

impl VideoFormat {
    pub(crate) fn id(self) -> u32 {
        match self {
            Self::Rgbx => 7,
            Self::Bgrx => 8,
            Self::Rgba => 11,
            Self::Bgra => 12,
        }
    }

    pub(crate) fn from_id(id: u32) -> Option<Self> {
        [Self::Rgbx, Self::Bgrx, Self::Rgba, Self::Bgra]
            .into_iter()
            .find(|format| format.id() == id)
    }

    /// Red and blue sit in the other order than in the program's BGRA.
    pub(crate) fn swaps_red_blue(self) -> bool {
        matches!(self, Self::Rgbx | Self::Rgba)
    }
}

fn padded(length: usize) -> usize {
    (length + 7) & !7
}

fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends a pod of `kind` with `body` and its padding.
fn pod(out: &mut Vec<u8>, kind: u32, body: &[u8]) {
    word(out, body.len() as u32);
    word(out, kind);
    out.extend_from_slice(body);
    out.resize(out.len() + padded(body.len()) - body.len(), 0);
}

fn id_pod(out: &mut Vec<u8>, id: u32) {
    pod(out, TYPE_ID, &id.to_le_bytes());
}

/// A choice pod: the kind, the child pod header and the values (each
/// `child_size` bytes, without padding between them).
fn choice(out: &mut Vec<u8>, choice_kind: u32, child_type: u32, child_size: u32, values: &[u32]) {
    let mut body = Vec::new();
    word(&mut body, choice_kind);
    word(&mut body, 0);
    word(&mut body, child_size);
    word(&mut body, child_type);
    for value in values {
        word(&mut body, *value);
    }
    pod(out, TYPE_CHOICE, &body);
}

fn property(out: &mut Vec<u8>, key: u32) {
    word(out, key);
    word(out, 0);
}

/// The `EnumFormat` pod that offers raw video as shared memory: BGRx first,
/// then RGBx, BGRA and RGBA; any size from 1 × 1 to 16384 × 16384 (default
/// 1920 × 1080); any frame rate from 0 to 1000 (default 0/1, variable).
/// There is no `VIDEO_modifier`, so producers choose shared memory over
/// DMA-BUF.
pub(crate) fn enum_format_pod() -> Vec<u8> {
    let mut body = Vec::new();
    word(&mut body, OBJECT_FORMAT);
    word(&mut body, PARAM_ENUM_FORMAT);
    property(&mut body, KEY_MEDIA_TYPE);
    id_pod(&mut body, MEDIA_TYPE_VIDEO);
    property(&mut body, KEY_MEDIA_SUBTYPE);
    id_pod(&mut body, MEDIA_SUBTYPE_RAW);
    property(&mut body, KEY_VIDEO_FORMAT);
    // The first value is the default, the following ones the alternatives;
    // the enum convention repeats the default.
    let formats = [
        VideoFormat::Bgrx,
        VideoFormat::Bgrx,
        VideoFormat::Rgbx,
        VideoFormat::Bgra,
        VideoFormat::Rgba,
    ]
    .map(VideoFormat::id);
    choice(&mut body, CHOICE_ENUM, TYPE_ID, 4, &formats);
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
    out
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

/// The value of a property: a plain value, or a choice of the kind `None`
/// (`spa_pod_fixate` turns every choice of the negotiated format into that
/// kind and leaves the values in place; the first one is the value). A
/// choice with alternatives (`Range`, `Enum`, ...) is an offer, no value.
fn plain_value(value_type: u32, value: &[u8]) -> Option<(u32, &[u8])> {
    if value_type != TYPE_CHOICE {
        return Some((value_type, value));
    }
    // choice kind, flags, size and type of the child, then the values.
    let (kind, child_size, child_type) = (
        read_u32(value, 0)?,
        read_u32(value, 8)? as usize,
        read_u32(value, 12)?,
    );
    if kind != CHOICE_NONE || child_type == TYPE_CHOICE {
        return None;
    }
    let first = value.get(16..16usize.checked_add(child_size)?)?;
    Some((child_type, first))
}

/// Reads `VIDEO_format` and `VIDEO_size` from a negotiated `Format` object
/// pod (plain values or fixated choices); all other properties are skipped
/// by their own sizes. `None` when the bytes are no such object or lack either value
/// or hold a format this reader does not know.
pub(crate) fn parse_video_format(bytes: &[u8]) -> Option<(VideoFormat, FrameSize)> {
    let size = read_u32(bytes, 0)? as usize;
    if read_u32(bytes, 4)? != TYPE_OBJECT || size < 8 {
        return None;
    }
    let body = bytes.get(8..8usize.checked_add(size)?)?;
    let (mut format, mut frame) = (None, None);
    let mut at = 8;
    while at + 16 <= body.len() {
        let key = read_u32(body, at)?;
        let value_size = read_u32(body, at + 8)? as usize;
        let value_type = read_u32(body, at + 12)?;
        let value = body.get(at + 16..at.checked_add(16)?.checked_add(value_size)?)?;
        let plain = if matches!(key, KEY_VIDEO_FORMAT | KEY_VIDEO_SIZE) {
            plain_value(value_type, value)
        } else {
            None
        };
        match (key, plain) {
            (KEY_VIDEO_FORMAT, Some((TYPE_ID, id))) => {
                format = VideoFormat::from_id(read_u32(id, 0)?);
            }
            (KEY_VIDEO_SIZE, Some((TYPE_RECTANGLE, rectangle))) => {
                frame = Some(FrameSize::new(
                    read_u32(rectangle, 0)?,
                    read_u32(rectangle, 4)?,
                ));
            }
            _ => {}
        }
        at = at.checked_add(16)?.checked_add(padded(value_size))?;
    }
    Some((format?, frame?))
}

fn int_property(body:&mut Vec<u8>,key:u32,value:u32) {
    property(body,key);pod(body,4,&value.to_le_bytes());
}

/// Functional consumer minimum, never an observed pool count or stride.
/// PipeWire 0.3.65 buffers.c parses buffers/stride as optional. Omitting the
/// count preserves its server fallback; a size range prevents the 8192-byte
/// fallback from becoming too small for an unconfigured video producer.
pub(crate) fn consumer_buffers_pod(size:FrameSize)->std::io::Result<Vec<u8>> {
    let minimum=size.width.checked_mul(size.height).and_then(|n|n.checked_mul(4))
        .filter(|n|*n!=0 && *n<=i32::MAX as u32)
        .ok_or_else(||std::io::Error::other("pipewire invalid consumer minimum"))?;
    let mut body=Vec::new();word(&mut body,0x40004);word(&mut body,5);
    int_property(&mut body,2,1);
    property(&mut body,3);choice(&mut body,CHOICE_RANGE,4,4,&[minimum,minimum,i32::MAX as u32]);
    property(&mut body,6);choice(&mut body,4,4,4,&[(1<<1)|(1<<2)]);
    let mut out=Vec::new();pod(&mut out,TYPE_OBJECT,&body);Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
fn word(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn pad(out: &mut Vec<u8>) {
    while out.len() % 8 != 0 {
        out.push(0);
    }
}

/// An `Id` pod: size 4, type 3, the id, 4 bytes of padding.
fn id_pod(out: &mut Vec<u8>, id: u32) {
    word(out, 4);
    word(out, 3);
    word(out, id);
    word(out, 0);
}

/// A choice pod over `values` of `child_size` bytes and `child_type`.
fn choice_pod(out: &mut Vec<u8>, kind: u32, child_size: u32, child_type: u32, values: &[u32]) {
    let body_size = 8 + 8 + values.len() as u32 * 4;
    word(out, body_size);
    word(out, 19);
    word(out, kind);
    word(out, 0);
    word(out, child_size);
    word(out, child_type);
    for value in values {
        word(out, *value);
    }
    pad(out);
}

#[test]
fn screen_task_spa_pod_enum_format_bytes() {
    // Written out from the description of `spa/param/video/format-utils.h`
    // and `spa/pod/pod.h`: object, then key/flags/pod for each property.
    let mut body = Vec::new();
    word(&mut body, 0x40003); // SPA_TYPE_OBJECT_Format
    word(&mut body, 3); // SPA_PARAM_EnumFormat
    word(&mut body, 1); // mediaType
    word(&mut body, 0);
    id_pod(&mut body, 2); // video
    word(&mut body, 2); // mediaSubtype
    word(&mut body, 0);
    id_pod(&mut body, 1); // raw
    word(&mut body, 0x20001); // VIDEO_format
    word(&mut body, 0);
    choice_pod(&mut body, 3, 4, 3, &[8, 8, 7, 12, 11]); // Enum of Ids
    word(&mut body, 0x20003); // VIDEO_size
    word(&mut body, 0);
    choice_pod(&mut body, 1, 8, 10, &[1920, 1080, 1, 1, 16384, 16384]); // Range
    word(&mut body, 0x20004); // VIDEO_framerate
    word(&mut body, 0);
    choice_pod(&mut body, 1, 8, 11, &[0, 1, 0, 1, 1000, 1]);
    let mut expected = Vec::new();
    word(&mut expected, body.len() as u32);
    word(&mut expected, 15); // Object
    expected.extend_from_slice(&body);
    pad(&mut expected);
    let built = enum_format_pod();
    assert_eq!(built, expected);
    let size = u32::from_le_bytes(built[0..4].try_into().unwrap()) as usize;
    assert_eq!(size, built.len() - 8);
    assert_eq!(built.len() % 8, 0);
    // No VIDEO_modifier (key 0x20005): producers keep to shared memory.
    assert!(!built
        .windows(4)
        .any(|word| word == 0x20005u32.to_le_bytes()));
}

#[test]
fn spa_pod_negotiated_format_is_read_and_garbage_is_not() {
    let mut body = Vec::new();
    word(&mut body, 0x40003);
    word(&mut body, 4); // SPA_PARAM_Format
    word(&mut body, 1);
    word(&mut body, 0);
    id_pod(&mut body, 2);
    word(&mut body, 0x20001);
    word(&mut body, 0);
    id_pod(&mut body, 12); // BGRA
    word(&mut body, 0x20003);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 10); // Rectangle
    word(&mut body, 1920);
    word(&mut body, 1080);
    word(&mut body, 0x20004);
    word(&mut body, 0);
    word(&mut body, 8);
    word(&mut body, 11); // Fraction
    word(&mut body, 60);
    word(&mut body, 1);
    let mut pod = Vec::new();
    word(&mut pod, body.len() as u32);
    word(&mut pod, 15);
    pod.extend_from_slice(&body);
    assert_eq!(
        parse_video_format(&pod),
        Some((VideoFormat::Bgra, FrameSize::new(1920, 1080)))
    );
    // The offer holds choices, not values: nothing negotiated.
    assert_eq!(parse_video_format(&enum_format_pod()), None);
    assert_eq!(parse_video_format(&pod[..pod.len() - 5]), None);
    assert_eq!(parse_video_format(&[0u8; 3]), None);
    assert_eq!(parse_video_format(&[]), None);
}

/// `spa_pod_fixate` turns the choices of the negotiated format into choices
/// of the kind `None` that keep their values; the first one is the value
/// (KWin through the KDE portal). A choice that still offers alternatives is
/// no negotiated format.
#[test]
fn spa_pod_negotiated_format_reads_fixated_choices() {
    let object = |format_kind: u32, size_kind: u32| {
        let mut body = Vec::new();
        word(&mut body, 0x40003);
        word(&mut body, 4); // SPA_PARAM_Format
        word(&mut body, 0x20001);
        word(&mut body, 0);
        choice_pod(&mut body, format_kind, 4, 3, &[8, 12, 11]); // BGRx first
        word(&mut body, 0x20003);
        word(&mut body, 0);
        choice_pod(&mut body, size_kind, 8, 10, &[1280, 800, 1, 1, 4096, 4096]);
        let mut pod = Vec::new();
        word(&mut pod, body.len() as u32);
        word(&mut pod, 15);
        pod.extend_from_slice(&body);
        pod
    };
    assert_eq!(
        parse_video_format(&object(0, 0)),
        Some((VideoFormat::Bgrx, FrameSize::new(1280, 800)))
    );
    assert_eq!(parse_video_format(&object(3, 0)), None);
    assert_eq!(parse_video_format(&object(0, 1)), None);
}

}
