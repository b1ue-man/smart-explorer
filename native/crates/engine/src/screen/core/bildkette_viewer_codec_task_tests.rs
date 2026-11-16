//! The decoder side of the desktop viewer's picture chain (plan 2026-11-14
//! „Bildkette“ V1): hand-over of the decoded picture into returned storage
//! and the register's decisions about continuation and decoder switch.
use std::io;
use std::sync::Arc;

use crate::screen::core::codec_registry::{CodecRegistry, DecoderFactory};
use crate::screen::core::decoded_frame::DecodedFrame;
use crate::screen::core::frame_reference::ReferenceDecoder;
use crate::screen::core::host_backend::{
    Chroma, ScreenDecoder, ScreenEncoder, TilesScreenDecoder, TilesScreenEncoder, YuvPicture,
};
use crate::screen::core::negotiate::{CodecKind, CodecOffer, DecoderPreference, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize, YuvMatrix};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, FLAG_KEYFRAME, FLAG_MORE};

/// The records of one tile frame of `image`.
fn tile_records(
    encoder: &mut TilesScreenEncoder,
    image: &BgraImage,
    frame_id: u64,
    keyframe: bool,
) -> Vec<FrameRecord> {
    let encoded = encoder.encode(image, keyframe, 0).unwrap();
    let last = encoded.records.len().saturating_sub(1);
    encoded
        .records
        .into_iter()
        .enumerate()
        .map(|(index, payload)| FrameRecord {
            frame_id,
            capture_ts_us: 0,
            flags: if keyframe { FLAG_KEYFRAME } else { 0 }
                | if index < last { FLAG_MORE } else { 0 },
            payload,
        })
        .collect()
}

fn paint(image: &mut BgraImage, value: u8) {
    for (index, byte) in image.data_mut().iter_mut().enumerate() {
        *byte = if index % 4 == 3 { 0xFF } else { value };
    }
}

#[test]
fn bildkette_task_viewer_tiles_hand_over_into_returned_storage() {
    let size = FrameSize::new(64, 32);
    let mut image = BgraImage::new(size).unwrap();
    let mut encoder = TilesScreenEncoder::new(size).unwrap();
    let mut decoder = TilesScreenDecoder::new(size).unwrap();
    paint(&mut image, 40);
    for record in tile_records(&mut encoder, &image, 1, true) {
        decoder.decode(&record).unwrap();
    }
    // Without returned storage the hand-over is the copy it always was.
    let first = decoder.take_picture();
    assert_eq!(first.data(), image.data());
    let storage = first.data().as_ptr();
    decoder.recycle_picture(first);
    paint(&mut image, 90);
    for record in tile_records(&mut encoder, &image, 2, false) {
        decoder.decode(&record).unwrap();
    }
    // The tile picture is the reference of the next frame: it stays, and
    // its copy lands in the returned storage instead of a new allocation.
    let second = decoder.take_picture();
    assert_eq!(second.data().as_ptr(), storage, "no new picture allocated");
    assert_eq!(second.data(), image.data());
    assert_eq!(decoder.picture().data(), image.data());
    // Storage of another size is not used.
    decoder.recycle_picture(BgraImage::new(FrameSize::new(16, 16)).unwrap());
    let third = decoder.take_picture();
    assert_eq!(third.size(), size);
    assert!(!decoder.keeps_references(), "tiles never continue");
}

/// An inner decoder whose planes are handed over as they are.
struct Planes {
    out: Option<YuvPicture>,
    spare: Option<YuvPicture>,
    decoded: Option<DecodedFrame>,
    image: BgraImage,
}

impl ScreenDecoder for Planes {
    fn name(&self) -> String {
        "planes".into()
    }
    fn decode(&mut self, record: &FrameRecord) -> io::Result<bool> {
        let mut out = self.spare.take().unwrap_or(YuvPicture {
            y: vec![0; 8],
            y_stride: 4,
            uv: vec![0; 4],
            uv_stride: 4,
            chroma: Chroma::Nv12,
            matrix: YuvMatrix::Bt709,
        });
        out.y.fill(record.frame_id as u8);
        self.out = Some(out);
        self.decoded = Some(DecodedFrame::from_record(record));
        Ok(true)
    }
    fn decoded_frame(&self) -> Option<DecodedFrame> {
        self.decoded
    }
    fn picture(&self) -> &BgraImage {
        &self.image
    }
    fn yuv_picture(&self) -> Option<YuvPicture> {
        self.out.clone()
    }
    fn take_yuv_picture(&mut self) -> Option<YuvPicture> {
        self.out.take()
    }
    fn recycle_yuv(&mut self, planes: YuvPicture) {
        self.spare = Some(planes);
    }
    fn keeps_references(&self) -> bool {
        true
    }
}

#[test]
fn bildkette_task_viewer_reference_decoder_hands_the_planes_through() {
    let inner = Planes {
        out: None,
        spare: None,
        decoded: None,
        image: BgraImage::new(FrameSize::new(4, 2)).unwrap(),
    };
    let mut decoder = ReferenceDecoder::new(Box::new(inner));
    assert!(decoder.keeps_references(), "asks the inner decoder");
    assert!(decoder.take_yuv_picture().is_none(), "nothing valid yet");
    let record = |frame_id, flags| FrameRecord {
        frame_id,
        capture_ts_us: 0,
        flags,
        payload: vec![1],
    };
    assert!(decoder.decode(&record(1, FLAG_KEYFRAME)).unwrap());
    let planes = decoder.take_yuv_picture().expect("a valid picture");
    let storage = planes.y.as_ptr();
    decoder.recycle_yuv(planes);
    assert!(decoder.decode(&record(2, 0)).unwrap());
    let planes = decoder.take_yuv_picture().unwrap();
    assert_eq!(planes.y.as_ptr(), storage, "the inner decoder reused it");
    assert_eq!(planes.y, vec![2; 8]);
    // A refused record poisons the references: nothing is handed over.
    assert!(decoder.decode(&record(2, 0)).is_err());
    assert!(decoder.take_yuv_picture().is_none());

    // Video codecs check their own random-access mark, not H.264's IDR only.
    let video = |codec| {
        ReferenceDecoder::new_for_codec(
            Box::new(Planes {
                out: None,
                spare: None,
                decoded: None,
                image: BgraImage::new(FrameSize::new(4, 2)).unwrap(),
            }),
            codec,
        )
    };
    let key = |payload: &[u8]| FrameRecord {
        frame_id: 1,
        capture_ts_us: 0,
        flags: FLAG_KEYFRAME,
        payload: payload.to_vec(),
    };
    let mut hevc = video(CodecKind::Hevc);
    assert!(hevc.decode(&key(&[0, 0, 0, 1, 1 << 1, 1, 0x02])).is_err());
    let mut hevc = video(CodecKind::Hevc);
    assert!(hevc.decode(&key(&[0, 0, 0, 1, 19 << 1, 1, 0xaf])).unwrap());
    let mut av1 = video(CodecKind::Av1);
    assert!(av1.decode(&key(&[0x32, 2, 0b0010_0000, 0])).is_err());
    let mut av1 = video(CodecKind::Av1);
    assert!(av1.decode(&key(&[0x32, 2, 0, 0])).unwrap());
}

/// A decoder factory with fixed offers.
struct Offers(Vec<CodecOffer>, bool);

impl DecoderFactory for Offers {
    fn name(&self) -> String {
        "offers".into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        self.0.clone()
    }
    fn open(&self, _header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        Err(io::Error::other("not opened in this test"))
    }
    fn keeps_references(&self, codec: CodecKind) -> bool {
        self.1 && codec != CodecKind::H264
    }
}

#[test]
fn bildkette_task_viewer_register_decides_switch_and_announcement() {
    let small = CodecOffer::h264(H264Profile::High, false).with_max(FrameSize::new(1280, 720));
    let registry = CodecRegistry::new(
        Vec::new(),
        vec![
            Arc::new(Offers(vec![small], false)) as Arc<dyn DecoderFactory>,
            Arc::new(Offers(
                vec![CodecOffer::video(CodecKind::Av1, false, true)],
                true,
            )),
        ],
    );
    let h264 = (CodecKind::H264, Some(H264Profile::High));
    let hd = FrameSize::new(1280, 720);
    let uhd = FrameSize::new(3840, 2160);
    let soft = DecoderPreference::Software;
    let hard = DecoderPreference::Hardware;
    // Decoder switch: the stage and the stream's size must fit.
    assert!(registry.decodes_with(h264, hd, None, soft));
    assert!(!registry.decodes_with(h264, hd, None, hard));
    assert!(!registry.decodes_with(h264, uhd, None, soft));
    assert!(registry.decodes_with((CodecKind::Av1, None), uhd, None, hard));
    assert!(!registry.decodes_with((CodecKind::Av1, None), uhd, None, soft));
    assert!(registry.decodes_with((CodecKind::Tiles, None), uhd, None, soft));
    // Announcement: AV1 alone continues, H.264 among the offers does not.
    let auto = DecoderPreference::Automatic;
    let av1 = [
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::tiles(),
    ];
    assert!(registry.keeps_references(&av1, auto));
    assert!(!registry.keeps_references(&registry.decoder_offers(), auto));
    // An offer no decoder of the stage takes cannot be announced either.
    assert!(!registry.keeps_references(&av1, soft));
}
