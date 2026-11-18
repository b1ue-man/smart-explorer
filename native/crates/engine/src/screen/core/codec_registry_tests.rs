use super::*;
use crate::screen::core::negotiate::{negotiate_trusted, CodecPreference};

fn hardware(codecs: &[CodecKind]) -> Vec<CodecOffer> {
    codecs
        .iter()
        .map(|codec| match codec {
            CodecKind::H264 => CodecOffer::h264(H264Profile::High, true),
            CodecKind::Tiles => CodecOffer::tiles(),
            other => CodecOffer::video(*other, false, true),
        })
        .collect()
}

fn order(offers: &[CodecOffer], trust: CodecTrust, preference: CodecPreference) -> Vec<CodecKind> {
    negotiate_trusted(offers, offers, preference, None, trust)
        .iter()
        .map(|choice| choice.codec)
        .collect()
}

#[test]
fn codec_registry_task_automatic_never_puts_av1_first() {
    assert!(!keeps_first_rank(CodecKind::Av1));
    assert!(keeps_first_rank(CodecKind::Hevc) && keeps_first_rank(CodecKind::H264));
    let trust = first_rank_trust([CodecKind::H264, CodecKind::Hevc, CodecKind::Av1]);
    assert!(trust.trusts(CodecKind::Hevc) && !trust.trusts(CodecKind::Av1));
    let offers = hardware(&[CodecKind::Av1, CodecKind::Hevc, CodecKind::H264]);
    // A checked HEVC first, H.264, then AV1, the tiles last.
    assert_eq!(
        order(&offers, trust, CodecPreference::Auto),
        [
            CodecKind::Hevc,
            CodecKind::H264,
            CodecKind::Av1,
            CodecKind::Tiles
        ]
    );
    // AV1 alone checked: H.264 first.
    let only_av1 = first_rank_trust([CodecKind::Av1]);
    assert_eq!(
        order(&offers, only_av1, CodecPreference::Auto)[0],
        CodecKind::H264
    );
    // Named by the user, AV1 stays selectable.
    assert_eq!(
        order(&offers, trust, CodecPreference::Av1),
        [CodecKind::Av1]
    );
}

fn row(codec: CodecKind, hardware: bool, profiles: &[&str], state: Option<&str>) -> CodecEntry {
    CodecEntry {
        codec,
        profiles: Vec::new(),
        name: format!("{}-{hardware}", codec.key()),
        hardware,
        profile_names: profiles.iter().map(|name| name.to_string()).collect(),
        refused_settings: Vec::new(),
        verdict: state.map(|state| VerdictView {
            state: state.into(),
            reason: None,
        }),
    }
}

#[test]
fn codec_registry_task_sharp_encoders_of_the_report() {
    let report = CodecReport {
        encoders: vec![
            row(CodecKind::Av1, true, &["main"], Some("verified")),
            row(
                CodecKind::Hevc,
                true,
                &["main", "main_444"],
                Some("verified"),
            ),
            row(CodecKind::H264, true, &["high"], None),
            row(CodecKind::H264, false, &["cb"], None),
            row(CodecKind::Hevc, true, &["main"], Some("failed")),
            tiles_entry(),
        ],
        decoders: Vec::new(),
        openh264: OpenH264Report::new(OpenH264State::Absent, None),
    };
    let codecs: Vec<CodecKind> = report
        .sharp_encoders()
        .iter()
        .map(|entry| entry.codec)
        .collect();
    assert_eq!(codecs, [CodecKind::Hevc, CodecKind::H264]);
    assert!(report.sharp_444());
    let amd = CodecReport {
        encoders: vec![
            row(CodecKind::Hevc, true, &["main"], Some("verified")),
            tiles_entry(),
        ],
        ..report
    };
    assert!(!amd.sharp_444());
    assert_eq!(amd.sharp_encoders().len(), 1);
}
