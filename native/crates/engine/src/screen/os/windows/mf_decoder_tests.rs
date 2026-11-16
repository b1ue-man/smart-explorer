use crate::screen::core::host_backend::{ScreenDecoder, ScreenEncoder};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::synthetic::{read_barcode, render, BarcodeRead};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

/// The software and every hardware encoder of Media Foundation against
/// its decoder, both profiles: 30 frames of the test picture must come
/// back with their barcodes. Only where Media Foundation runs; the
/// GitHub runners have none (K6), the suite reports it as not run there.
#[test]
#[ignore = "needs Media Foundation; the suite runs it where `se screen codec` lists an MFT; Suite rechte-desktop-tresore-task"]
fn screen_task_mf_encode_decode() {
    assert!(super::super::mf_common::available(), "no Media Foundation");
    let (encoders, decoders) = super::super::mf_common::codecs();
    let size = FrameSize::new(640, 360);
    let decoder_factory = decoders.first().expect("an MF decoder");
    for factory in &encoders {
        for profile in [H264Profile::ConstrainedBaseline, H264Profile::High] {
            let hardware = factory.offers().iter().any(|offer| offer.hardware);
            let choice = CodecChoice {
                codec: CodecKind::H264,
                profile: Some(profile),
                encoder_hardware: hardware,
                decoder_hardware: false,
                max_size: FrameSize::MAX,
                max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
                max_macroblocks_per_second: None,
            };
            let mut encoder = factory.open(&choice, size, 30).unwrap();
            let header = GopHeader {
                family_layout_revision: None,
                frame_contract: None,
                gop_id: 1,
                kind: StreamKind::Video,
                codec: CodecKind::H264,
                profile: Some(profile),
                size,
            };
            let mut decoder = decoder_factory.open(&header).unwrap();
            let mut image = BgraImage::new(size).unwrap();
            let mut good = 0;
            for frame_id in 1..=30u64 {
                render(frame_id, &mut image).unwrap();
                let encoded = encoder.encode(&image, frame_id == 1, 2_000_000).unwrap();
                if frame_id == 1 {
                    assert!(encoded.keyframe, "the first frame is a key frame");
                }
                for payload in encoded.records {
                    let flags = if encoded.keyframe { FLAG_KEYFRAME } else { 0 };
                    let record = FrameRecord {
                        frame_id,
                        capture_ts_us: 0,
                        flags,
                        payload,
                    };
                    if decoder.decode(&record).unwrap() {
                        let output = decoder
                            .decoded_frame()
                            .expect("decoded MF picture identity");
                        let read = read_barcode(&decoder.picture().view(), size).unwrap();
                        if matches!(read, BarcodeRead::Valid(id) if id == output.frame_id) {
                            good += 1;
                        }
                    }
                }
            }
            assert!(good >= 25, "{}: {good} of 30 frames back", encoder.name());
        }
    }
}
