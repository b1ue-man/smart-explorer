use super::H264Profile::{ConstrainedBaseline as Cb, High};
use super::*;

fn hardware(profile: H264Profile) -> CodecOffer {
    CodecOffer::h264(profile, true)
}

fn software(profile: H264Profile) -> CodecOffer {
    CodecOffer::h264(profile, false)
}

type Row = (CodecKind, Option<H264Profile>, bool, bool);

fn rows(choices: &[CodecChoice]) -> Vec<Row> {
    choices
        .iter()
        .map(|choice| {
            (
                choice.codec,
                choice.profile,
                choice.encoder_hardware,
                choice.decoder_hardware,
            )
        })
        .collect()
}

const TILES: Row = (CodecKind::Tiles, None, false, false);

#[test]
fn screen_task_negotiate_matrix() {
    use CodecPreference::{Auto, Tiles, H264};

    // The full tier order: hardware/hardware, hardware/software,
    // software/hardware, software/software, then tiles.
    let host = [software(Cb), hardware(Cb)];
    let viewer = [hardware(Cb), software(Cb)];
    assert_eq!(
        rows(&negotiate(&host, &viewer, Auto)),
        vec![
            (CodecKind::H264, Some(Cb), true, true),
            (CodecKind::H264, Some(Cb), true, false),
            (CodecKind::H264, Some(Cb), false, true),
            (CodecKind::H264, Some(Cb), false, false),
            TILES,
        ]
    );

    // Within a tier `high` comes before `cb`; a lower tier never jumps ahead.
    let host = [software(High), hardware(Cb), hardware(High)];
    let viewer = [hardware(High)];
    assert_eq!(
        rows(&negotiate(&host, &viewer, Auto)),
        vec![
            (CodecKind::H264, Some(High), true, true),
            (CodecKind::H264, Some(Cb), true, true),
            (CodecKind::H264, Some(High), false, true),
            TILES,
        ]
    );

    // An OpenH264 viewer lists `cb` and `high` (A4); duplicates collapse.
    let host = [
        hardware(Cb),
        software(Cb),
        software(High),
        CodecOffer::tiles(),
    ];
    let viewer = [software(Cb), software(High), CodecOffer::tiles()];
    assert_eq!(
        rows(&negotiate(&host, &viewer, Auto)),
        vec![
            (CodecKind::H264, Some(Cb), true, false),
            (CodecKind::H264, Some(High), false, false),
            (CodecKind::H264, Some(Cb), false, false),
            TILES,
        ]
    );

    // A High decoder takes Constrained Baseline, a cb-only decoder not High.
    assert_eq!(
        rows(&negotiate(&[hardware(Cb)], &[hardware(High)], Auto)),
        vec![(CodecKind::H264, Some(Cb), true, true), TILES]
    );
    assert_eq!(
        rows(&negotiate(&[hardware(High)], &[hardware(Cb)], Auto)),
        vec![TILES]
    );

    // Tiles are part of protocol version 1: they remain without any list.
    assert_eq!(rows(&negotiate(&[], &[], Auto)), vec![TILES]);
    let without_profile = CodecOffer {
        profile: None,
        ..hardware(High)
    };
    assert_eq!(
        rows(&negotiate(&[without_profile], &[hardware(High)], Auto)),
        vec![TILES]
    );

    // Forced codecs (`--codec`, „Verlustfrei“, group previews).
    let host = [hardware(High), software(Cb)];
    let viewer = [hardware(High), software(Cb)];
    assert_eq!(rows(&negotiate(&host, &viewer, Tiles)), vec![TILES]);
    let forced = rows(&negotiate(&host, &viewer, H264));
    assert_eq!(
        forced.first(),
        Some(&(CodecKind::H264, Some(High), true, true))
    );
    assert!(forced.iter().all(|row| row.0 == CodecKind::H264));
    assert!(negotiate(&[hardware(High)], &[hardware(Cb)], H264).is_empty());

    // The size limit is the smaller of both sides, never above 4096 × 2304.
    let host = [hardware(High).with_max(FrameSize::new(1920, 1080))];
    let viewer = [software(High).with_max(FrameSize::new(3840, 1000))];
    let choices = negotiate(&host, &viewer, Auto);
    assert_eq!(choices[0].max_size, FrameSize::new(1920, 1000));
    assert_eq!(choices[1].max_size, FrameSize::MAX);
    let huge = [CodecOffer::tiles().with_max(FrameSize::new(8192, 8192))];
    assert_eq!(negotiate(&huge, &[], Tiles)[0].max_size, FrameSize::MAX);
    let small = [CodecOffer::tiles().with_max(FrameSize::new(320, 180))];
    assert_eq!(
        negotiate(&[], &small, Tiles)[0].max_size,
        FrameSize::new(320, 180)
    );

    // Offers beyond 32 per side are ignored.
    let mut many = vec![software(Cb); MAX_CODEC_OFFERS];
    many.push(hardware(High));
    assert_eq!(
        rows(&negotiate(&many, &[hardware(High)], Auto)),
        vec![(CodecKind::H264, Some(Cb), false, true), TILES]
    );

    // Names and wire codes.
    for (text, preference) in [("auto", Auto), ("h264", H264), ("tiles", Tiles)] {
        assert_eq!(CodecPreference::parse(text), Some(preference));
        assert_eq!(preference.key(), text);
    }
    assert_eq!(CodecPreference::parse("vp9"), None);
    for codec in [CodecKind::H264, CodecKind::Tiles] {
        assert_eq!(CodecKind::from_wire_code(codec.wire_code()), Some(codec));
    }
    for profile in [Cb, High] {
        assert_eq!(
            H264Profile::from_wire_code(profile.wire_code()),
            Some(profile)
        );
    }
    assert_eq!(CodecKind::from_wire_code(3), None);
    assert_eq!(
        serde_json::to_string(&hardware(Cb)).unwrap(),
        r#"{"codec":"h264","profile":"cb","hardware":true}"#
    );
}
