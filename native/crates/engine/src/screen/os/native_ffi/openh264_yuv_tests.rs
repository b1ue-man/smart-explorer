//! `prefer_yuv` of the OpenH264 decoder (SCR-M7, U25-5). Needs the pinned
//! Cisco library like the other OpenH264 tests: `SE_SCREEN_TASK_OPENH264`
//! names the `.bz2` the suite downloads. The stream is made by the OpenH264
//! encoder of the same library, so no second test tool is needed.
use std::sync::PoisonError;

use super::tests::{openh264_registry, store_with_openh264};
use super::TEST_LOCK;
use crate::screen::core::host_backend::{Chroma, ScreenDecoder};
use crate::screen::core::negotiate::{negotiate, CodecKind, CodecPreference};
use crate::screen::core::pixel_convert::{
    i420_to_bgra, BgraImage, FrameSize, I420Image, YuvMatrix,
};
use crate::screen::core::synthetic::render;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

fn decode_all(decoder: &mut dyn ScreenDecoder, payloads: &[Vec<u8>]) -> bool {
    let mut shown = false;
    for payload in payloads {
        let record = FrameRecord {
            frame_id: 1,
            capture_ts_us: 0,
            flags: FLAG_KEYFRAME,
            payload: payload.clone(),
        };
        shown |= decoder.decode(&record).unwrap();
    }
    shown
}

#[test]
#[ignore = "needs SE_SCREEN_TASK_OPENH264 (the suite downloads the pinned file)"]
fn screen_task_decoder_prefer_yuv_skips_bgra() {
    let _lock = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let (_dir, store) = store_with_openh264();
    let codecs = openh264_registry(&store);
    let choice = negotiate(
        &codecs.encoder_offers(),
        &codecs.decoder_offers(),
        CodecPreference::H264,
    )[0];
    assert_eq!(choice.codec, CodecKind::H264);
    let size = FrameSize::new(640, 360);
    let mut opened = codecs.open_encoder(&choice, size, 30).unwrap();
    let mut image = BgraImage::new(size).unwrap();
    render(1, &mut image).unwrap();
    let encoded = opened.encoder.encode(&image, true, 2_000_000).unwrap();
    assert!(encoded.keyframe && !encoded.records.is_empty());
    let header = GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 1,
        kind: StreamKind::Video,
        codec: CodecKind::H264,
        profile: choice.profile,
        size,
    };

    // Without the request the decoder converts and hands out no planes.
    let mut plain = codecs.open_decoder(&header).unwrap();
    assert!(decode_all(plain.as_mut(), &encoded.records));
    assert!(plain.yuv_picture().is_none());
    let converted = plain.picture().data().to_vec();
    assert!(
        converted
            .chunks_exact(4)
            .any(|pixel| pixel[..3] != converted[..3]),
        "the test picture has content"
    );

    // With the request it keeps the planes and leaves the BGRA picture black.
    let mut planes_only = codecs.open_decoder(&header).unwrap();
    planes_only.prefer_yuv(true);
    assert!(decode_all(planes_only.as_mut(), &encoded.records));
    assert!(
        planes_only
            .picture()
            .data()
            .chunks_exact(4)
            .all(|pixel| pixel[..3] == [0, 0, 0]),
        "the conversion to BGRA must be skipped"
    );
    let picture = planes_only.yuv_picture().expect("planes after prefer_yuv");
    assert_eq!(picture.matrix, YuvMatrix::Bt709);
    assert_eq!(picture.y_stride, size.width as usize);
    let chroma = size.chroma();
    assert_eq!(picture.uv_stride, chroma.width as usize);
    let Chroma::I420 { u, v } = picture.chroma else {
        panic!("OpenH264 delivers I420");
    };
    assert_eq!(picture.y.len(), size.pixels() as usize);
    assert_eq!(
        (u.len(), v.len()),
        (chroma.pixels() as usize, chroma.pixels() as usize)
    );

    // The planes are the ones the plain path converted.
    let mut again = BgraImage::new(size).unwrap();
    let planes = I420Image {
        size,
        y: picture.y,
        u,
        v,
    };
    i420_to_bgra(&planes, YuvMatrix::Bt709, &mut again).unwrap();
    assert_eq!(again.data(), converted.as_slice());

    // Switched off again, the decoder converts and keeps no planes.
    planes_only.prefer_yuv(false);
    assert!(planes_only.yuv_picture().is_none());
    assert!(decode_all(planes_only.as_mut(), &encoded.records));
    assert_eq!(planes_only.picture().data(), converted.as_slice());
}
