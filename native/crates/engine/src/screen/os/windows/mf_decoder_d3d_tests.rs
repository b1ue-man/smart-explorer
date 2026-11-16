use super::super::{DecoderPreference, MfDecoder};
use crate::screen::core::host_backend::{ScreenDecoder, ScreenEncoder};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, H264Profile};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::synthetic::{read_barcode, render, BarcodeRead};
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME};

/// The first Media Foundation encoder against the decoder on the
/// hardware stage and on the software stage: both name their stage and
/// bring 25 of 30 pictures back with their barcodes, as planes (the
/// viewer's GPU path) and as BGRA. Needs a GPU with an H.264 decoder.
#[test]
#[ignore = "needs Media Foundation and a D3D11 H.264 video decoder; Suite keine (kein Runner mit Grafikkarte)"]
fn bildkette_task_win_decode_hardware_roundtrip() {
    let mf = super::super::super::mf_common::available();
    assert!(mf, "no Media Foundation");
    assert!(
        super::hardware_offered(CodecKind::H264),
        "no D3D11 H.264 decoder"
    );
    let (encoders, decoders) = super::super::super::mf_common::codecs();
    let factory = encoders.first().expect("an MF encoder");
    let name = decoders.first().expect("an MF decoder").name();
    // BK6: the decoder lists its hardware and its software stage, so the
    // viewer's choice „Dekodierer“ can keep either.
    let offers = decoders[0].offers();
    assert!(offers.iter().any(|offer| offer.hardware));
    assert!(offers.iter().any(|offer| !offer.hardware));
    let size = FrameSize::new(1280, 720);
    let profile = H264Profile::High;
    for (preference, label, yuv) in [
        (DecoderPreference::Hardware, "Hardware", true),
        (DecoderPreference::Hardware, "Hardware", false),
        (DecoderPreference::Software, "Software", true),
    ] {
        let choice = CodecChoice {
            codec: CodecKind::H264,
            profile: Some(profile),
            encoder_hardware: false,
            decoder_hardware: preference == DecoderPreference::Hardware,
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
        let mut decoder = MfDecoder::open(0, &name, &header, preference).unwrap();
        assert!(decoder.name().contains(label), "{}", decoder.name());
        decoder.prefer_yuv(yuv);
        let mut image = BgraImage::new(size).unwrap();
        let mut good = 0;
        for frame_id in 1..=30u64 {
            render(frame_id, &mut image).unwrap();
            let encoded = encoder.encode(&image, frame_id == 1, 4_000_000).unwrap();
            for payload in encoded.records {
                let flags = if encoded.keyframe { FLAG_KEYFRAME } else { 0 };
                let record = FrameRecord {
                    frame_id,
                    capture_ts_us: 0,
                    flags,
                    payload,
                };
                if !decoder.decode(&record).unwrap() {
                    continue;
                }
                let output = decoder.decoded_frame().expect("picture identity");
                if yuv {
                    let planes = decoder.yuv_picture().expect("NV12 planes");
                    assert!(planes.y_stride >= size.width as usize);
                    assert!(planes.y.len() >= planes.y_stride * size.height as usize);
                    good += 1;
                } else {
                    let read = read_barcode(&decoder.picture().view(), size).unwrap();
                    if matches!(read, BarcodeRead::Valid(id) if id == output.frame_id) {
                        good += 1;
                    }
                }
            }
        }
        assert!(good >= 25, "{}: {good} of 30 pictures", decoder.name());
    }
}
