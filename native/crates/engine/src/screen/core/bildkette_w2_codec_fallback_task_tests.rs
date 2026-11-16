//! Task suite „Bildkette“ merge W2 (all platforms; bk7b-bericht U1): when
//! the encoder of the session's codec fails and no other encoder of that
//! codec is left, the host takes the next negotiated pairing of another
//! video codec (HEVC → H.264) before the lossless tiles; the viewer learns
//! the codec from the header of the new picture group. A viewer of 0.5.196
//! negotiates H.264 and the tiles only and keeps H.264 → tiles. Not
//! registered yet (`docs/tests/register.tsv`): the manager adds the rows.
use super::*;
use crate::screen::core::negotiate::{negotiate, CodecOffer, CodecPreference, H264Profile};
use crate::screen::core::wire_binary::GopHeader;

fn codecs(choices: &[CodecChoice]) -> Vec<CodecKind> {
    choices.iter().map(|choice| choice.codec).collect()
}

#[test]
fn bildkette_task_linux_w2_codec_falls_back_in_the_session_before_the_tiles() {
    let encoders = [
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Hevc, true, true),
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::tiles(),
    ];
    // A new viewer named HEVC in `decoders_v2`: HEVC first (B-30, hardware
    // on both sides), H.264 the next codec; 4:4:4 and the tiles never.
    let new_viewer = [
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Hevc, true, true),
        CodecOffer::tiles(),
    ];
    let choices = negotiate(&encoders, &new_viewer, CodecPreference::Auto);
    let choice = choices[0];
    assert_eq!(choice.codec, CodecKind::Hevc);
    assert!(!choice.codec.chroma_444(choice.profile));
    let fallbacks = codec_fallbacks(&choices, &choice);
    assert_eq!(codecs(&fallbacks), [CodecKind::H264]);
    assert_eq!(fallbacks[0].profile, Some(H264Profile::High));
    // The choice of an H.264 session has nothing better left but HEVC.
    let h264 = choices
        .iter()
        .copied()
        .find(|choice| choice.codec == CodecKind::H264)
        .unwrap();
    assert_eq!(codecs(&codec_fallbacks(&choices, &h264)), [CodecKind::Hevc]);
    // A viewer of 0.5.196 (`decoders` with H.264 and the tiles only).
    let old_viewer = [
        CodecOffer::h264(H264Profile::High, false),
        CodecOffer::tiles(),
    ];
    let choices = negotiate(&encoders, &old_viewer, CodecPreference::Auto);
    assert_eq!(choices[0].codec, CodecKind::H264);
    assert!(codec_fallbacks(&choices, &choices[0]).is_empty());
    // Successive picture groups of the session name their codec in the
    // header the viewer opens its decoder from (`CodecRegistry::open_decoder`).
    for (gop_id, codec, profile) in [
        (1, CodecKind::Hevc, None),
        (2, CodecKind::H264, Some(H264Profile::High)),
        (3, CodecKind::Tiles, None),
    ] {
        let header = GopHeader {
            gop_id,
            frame_contract: None,
            family_layout_revision: None,
            kind: StreamKind::Video,
            codec,
            profile,
            size: FrameSize::new(1280, 720),
        };
        let decoded = GopHeader::decode(&header.encode().unwrap()).unwrap();
        assert_eq!((decoded.gop_id, decoded.codec), (gop_id, codec));
    }
    let opener = include_str!("codec_registry_open.rs");
    assert!(opener.contains("pub fn open_decoder(&self, header: &GopHeader)"));
    // The pipeline asks for the next codec before it takes the tiles, and
    // the new choice starts a new picture group (`invalidate_reference`).
    let produce = include_str!("host_pipeline_produce.rs");
    let next = produce.find("self.fall_back_to_next_codec()").unwrap();
    let tiles = produce.find("encoder_fallback_to_tiles").unwrap();
    assert!(next < tiles);
    let quality = include_str!("host_quality.rs");
    let switch = quality.find("fn fall_back_to_next_codec").unwrap();
    assert!(quality[switch..].contains("self.invalidate_reference();"));
    let server = include_str!("host_server.rs");
    assert!(server.contains("pipeline.set_codec_fallbacks(fallbacks);"));
}
