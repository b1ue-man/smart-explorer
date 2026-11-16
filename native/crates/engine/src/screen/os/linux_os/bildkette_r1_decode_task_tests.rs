//! Task suite „Bildkette“ r1, Linux viewer without GPU or dav1d: the VA-API
//! decoders hand their planes over and take given-back planes as the
//! storage of the next picture (`DecodeOutput`, H.264, HEVC and AV1 share
//! it); the reference bookkeeping of the VA-API HEVC (POC buffer) and AV1
//! (eight slots) decoders still holds a confirmed picture when the pictures
//! after it were lost, so both, and dav1d, announce the continuation (B-36)
//! per codec, H.264 never. Not registered yet (`docs/tests/register.tsv`):
//! the manager adds the rows.
use std::sync::Arc;

use super::super::av1_soft_codec::Dav1dFactory;
use super::super::bildkette_hw_hevc_task_tests::{maps, picture, tools, SIZE};
use super::super::h264_bits::nal_units;
use super::super::vaapi_av1_decode::refresh;
use super::super::vaapi_av1_frame::FrameHeader;
use super::super::vaapi_av1_syntax::{RefSlot, NUM_REF_FRAMES};
use super::super::vaapi_ffi::VA_ATTRIB_NOT_SUPPORTED;
use super::super::vaapi_hevc_dpb::Dpb;
use super::super::vaapi_hevc_headers::{parameter_sets, slice_header, HevcLayout};
use super::super::vaapi_hevc_slice::parse_slice;
use super::super::vaapi_hevc_syntax::{nal_header, ShortTermRps};
use super::{continues, DecodeOutput};
use crate::screen::core::codec_registry::{CodecRegistry, DecoderFactory};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::Chroma;
use crate::screen::core::negotiate::{CodecKind, CodecOffer, DecoderPreference, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

#[test]
fn bildkette_task_r1_linux_decoders_hand_their_planes_over() {
    let size = FrameSize::new(64, 32);
    let (luma, chroma) = (64 * 32, 64 * 16);
    let mut output = DecodeOutput::new(size, FrameLimits::LEGACY).unwrap();
    output.prefer_yuv(true);
    output.y = vec![1; luma];
    output.uv = vec![2; chroma];
    output.publish().unwrap();
    let first = output.take_yuv_picture().expect("planes handed over");
    assert_eq!((first.y.len(), first.y[0], first.uv[0]), (luma, 1, 2));
    assert_eq!((first.y_stride, first.uv_stride), (64, 64));
    assert!(matches!(first.chroma, Chroma::Nv12));
    assert!(output.take_yuv_picture().is_none(), "handed over once");
    assert!(output.yuv_picture().is_none(), "no copy left behind");
    // Given back: the next read fills that storage, no new allocation.
    let storage = first.y.as_ptr();
    output.recycle_yuv(first);
    output.y = vec![3; luma];
    output.uv = vec![4; chroma];
    output.publish().unwrap();
    assert_eq!(output.y.as_ptr(), storage, "recycled planes are reused");
    let second = output.take_yuv_picture().unwrap();
    assert_eq!((second.y[0], second.uv[0]), (3, 4));
    // Without the plane path given-back planes are dropped.
    output.prefer_yuv(false);
    output.recycle_yuv(second);
    assert!(output.spare.is_none());
    assert!(output.take_yuv_picture().is_none());
}

#[test]
fn bildkette_task_r1_linux_hevc_continuation_finds_the_confirmed_picture() {
    let layout = HevcLayout::new(SIZE, VA_ATTRIB_NOT_SUPPORTED);
    let tools = tools();
    let (sps_map, pps_map) = maps(&nal_units(&parameter_sets(&layout, &tools, 123)));
    let sps = &sps_map[&0];
    let header = |idr: bool, poc: u32| {
        let written = slice_header(&tools, &picture(idr, poc));
        let unit = nal_units(&written)[0].to_vec();
        let nal_type = nal_header(&unit).unwrap().0;
        parse_slice(&unit, nal_type, &sps_map, &pps_map, None).unwrap()
    };
    let mut dpb = Dpb::default();
    for poc in 0..4u32 {
        let h = header(poc == 0, poc);
        let counted = dpb.poc(&h, sps, 0).unwrap();
        dpb.apply_rps(&h, counted).unwrap();
        dpb.insert(100 + poc, counted, sps).unwrap();
    }
    // Pictures 4 and 5 never reach the viewer; the host continues from the
    // confirmed picture 3: used, the lost ones named but not used.
    let mut next = header(false, 6);
    next.rps = ShortTermRps {
        negative: vec![(-1, false), (-2, false), (-3, true)],
        positive: Vec::new(),
    };
    let poc = dpb.poc(&next, sps, 0).unwrap();
    assert_eq!(poc, 6, "the POC counts on over the gap");
    let refs = dpb.apply_rps(&next, poc).unwrap();
    assert_eq!(refs.before, vec![3]);
    assert_eq!(refs.frames.len(), 1);
    assert_eq!(refs.frames[0].picture_id, 103);
    assert!(dpb.holds(103) && !dpb.holds(102));
    // A continuation that names a lost picture is an error, never a wrong
    // picture: the viewer asks for a key frame.
    let mut lost = header(false, 7);
    lost.rps = ShortTermRps {
        negative: vec![(-2, true)],
        positive: Vec::new(),
    };
    let poc = dpb.poc(&lost, sps, 0).unwrap();
    assert!(dpb.apply_rps(&lost, poc).is_err());
}

#[test]
fn bildkette_task_r1_linux_av1_slots_change_only_with_decoded_frames() {
    let mut slots: [RefSlot; NUM_REF_FRAMES] = Default::default();
    let frame = |refresh_frame_flags: u8, order_hint: u32| FrameHeader {
        refresh_frame_flags,
        order_hint,
        ..FrameHeader::default()
    };
    refresh(&mut slots, &frame(0xff, 0), 10);
    refresh(&mut slots, &frame(0x01, 1), 11);
    // The viewer confirms the frame in slot 1; a later frame that would
    // refresh slot 1 again is lost and never reaches `refresh`.
    refresh(&mut slots, &frame(0x02, 2), 12);
    assert!(slots[1].valid);
    assert_eq!((slots[1].surface, slots[1].order_hint), (12, 2));
    assert_eq!(slots[0].surface, 11);
    assert!(slots[2..]
        .iter()
        .all(|slot| slot.valid && slot.surface == 10));
}

#[test]
fn bildkette_task_r1_linux_continuation_is_announced_per_codec() {
    assert!(continues(CodecKind::Hevc) && continues(CodecKind::Av1));
    assert!(!continues(CodecKind::H264) && !continues(CodecKind::Tiles));
    assert!(Dav1dFactory.keeps_references(CodecKind::Av1));
    assert!(!Dav1dFactory.keeps_references(CodecKind::H264));
    // The register announces AV1 (`gop_continue_decoders`, P1), not H.264.
    let registry = CodecRegistry::new(
        Vec::new(),
        vec![Arc::new(Dav1dFactory) as Arc<dyn DecoderFactory>],
    );
    let av1 = CodecOffer::video(CodecKind::Av1, false, false);
    let h264 = CodecOffer::h264(H264Profile::High, false);
    let auto = DecoderPreference::Automatic;
    assert_eq!(registry.continuing_offers(&[av1, h264], auto), vec![av1]);
    assert!(registry.keeps_references(&[av1], auto));
    assert!(!registry.keeps_references(&[av1, h264], auto));
}
