//! The lists offer only what works for the pair of devices (user rule of
//! 2026-11-24): host encoders × viewer decoders → usable entries with the
//! German reason of every other one, and a stored choice the host cannot
//! serve runs as „Automatisch“ for that session only.
use super::*;
use crate::screen::core::negotiate::H264Profile;

/// A Windows host with an NVIDIA RTX A4500: NVENC H.264 and HEVC (4:2:0
/// and 4:4:4), no AV1 encoder.
fn nvidia_host() -> Vec<CodecOffer> {
    vec![
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Hevc, true, true),
        CodecOffer::tiles(),
    ]
}

/// A Linux host without GPU: the tiles, with OpenH264 also software H.264.
fn cpu_host(openh264: bool) -> Vec<CodecOffer> {
    let mut encoders = vec![CodecOffer::tiles()];
    if openh264 {
        encoders.insert(0, CodecOffer::h264(H264Profile::ConstrainedBaseline, false));
    }
    encoders
}

/// A viewer with hardware H.264, HEVC and AV1, software H.264 and the tiles.
fn viewer_offers() -> Vec<CodecOffer> {
    vec![
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::h264(H264Profile::High, false),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::tiles(),
    ]
}

fn viewer(decoders: &[CodecOffer]) -> Viewer<'_> {
    Viewer {
        decoders,
        decoder: DecoderPreference::Automatic,
        library: DecoderLibrary::Automatic,
    }
}

/// The reason of each „Codec“ entry, `None` for a usable one.
fn reasons(host: Option<&[CodecOffer]>, viewer: &Viewer) -> Vec<(CodecSetting, Option<String>)> {
    CodecSetting::ALL
        .map(|codec| {
            let verdict = codec_verdict(codec, host, viewer);
            (codec, verdict.reason().map(str::to_string))
        })
        .into()
}

fn blocked(codec: CodecSetting, reason: &str) -> (CodecSetting, Option<String>) {
    (codec, Some(reason.to_string()))
}

fn usable(codec: CodecSetting) -> (CodecSetting, Option<String>) {
    (codec, None)
}

/// Host encoders × viewer decoders → the entries of „Codec“: AV1 only with
/// an AV1 encoder at the host and an AV1 decoder here, „Scharf (GPU)“ only
/// with a GPU encoder of H.264 or HEVC, „Automatisch“ and „Verlustfrei“
/// always (the tiles); an unknown host leaves the named codecs unknown.
#[test]
fn codec_offer_task_codec_entries_follow_host_encoders_and_viewer_decoders() {
    use CodecSetting::*;
    let offers = viewer_offers();
    let all = viewer(&offers);
    let nvidia = nvidia_host();
    let no_av1 = "dieser Gastgeber hat keinen AV1-Kodierer";
    let expected = vec![
        usable(Auto),
        usable(H264),
        usable(Hevc),
        blocked(Av1, no_av1),
        usable(Sharp),
        usable(Lossless),
    ];
    assert_eq!(reasons(Some(&nvidia), &all), expected);

    let no_gpu = "dieser Gastgeber hat keinen GPU-Kodierer für H.264 oder HEVC";
    let tiles_only = cpu_host(false);
    let expected = vec![
        usable(Auto),
        blocked(H264, "dieser Gastgeber hat keinen H.264-Kodierer"),
        blocked(Hevc, "dieser Gastgeber hat keinen HEVC-Kodierer"),
        blocked(Av1, no_av1),
        blocked(Sharp, no_gpu),
        usable(Lossless),
    ];
    assert_eq!(reasons(Some(&tiles_only), &all), expected);
    // OpenH264 (Constrained Baseline) pairs with a `high` decoder.
    let openh264 = cpu_host(true);
    let got = reasons(Some(&openh264), &all);
    assert_eq!(got[1], usable(H264));
    assert_eq!(got[4], blocked(Sharp, no_gpu));

    // An AV1 encoder without an AV1 decoder on this device.
    let mut av1_host = nvidia_host();
    av1_host.push(CodecOffer::video(CodecKind::Av1, false, true));
    let no_av1_here: Vec<CodecOffer> = offers
        .iter()
        .filter(|offer| offer.codec != CodecKind::Av1)
        .copied()
        .collect();
    let got = reasons(Some(&av1_host), &viewer(&no_av1_here));
    assert_eq!(
        got[3],
        blocked(Av1, "dieses Gerät hat keinen AV1-Dekodierer")
    );
    assert_eq!(reasons(Some(&av1_host), &all)[3], usable(Av1));

    // Before the host's encoders are known only the tiles' entries are sure.
    for (codec, verdict) in CodecSetting::ALL.map(|c| (c, codec_verdict(c, None, &all))) {
        match codec {
            Auto | Lossless => assert_eq!(verdict, Verdict::Usable),
            _ => assert!(matches!(verdict, Verdict::Unknown(_)), "{codec:?}"),
        }
    }

    // „Dekodierer: Hardware“: the tiles decode in software only.
    let hardware = Viewer {
        decoder: DecoderPreference::Hardware,
        ..all
    };
    // „Verlustfrei“ works under every „Dekodierer“ and „Dekodier-Bibliothek“
    // (2026-11-24): those choose among video decoders only.
    let got = reasons(Some(&nvidia), &hardware);
    assert_eq!(got[5], usable(Lossless));
    assert_eq!(got[1], usable(H264));
    let ffmpeg = Viewer {
        library: DecoderLibrary::Ffmpeg,
        ..all
    };
    assert_eq!(reasons(Some(&nvidia), &ffmpeg)[5], usable(Lossless));
    // The request's decoders: „Scharf (GPU)“ names H.264 and HEVC alone.
    let sharp = named_decoders(Sharp, &all);
    assert!(sharp
        .iter()
        .all(|offer| matches!(offer.codec, CodecKind::H264 | CodecKind::Hevc)));
    assert_eq!(named_decoders(Lossless, &all), vec![CodecOffer::tiles()]);
}

/// A stored choice the host cannot serve: the session runs with
/// „Automatisch“ in its place, says why, and keeps what the host serves.
#[test]
fn codec_offer_task_stored_choice_the_host_cannot_serve_runs_automatic() {
    use CodecSetting::*;
    let offers = viewer_offers();
    let all = viewer(&offers);
    let nvidia = nvidia_host();
    let choice = session_choice(Av1, Some(&nvidia), &all);
    assert_eq!(choice.codec, Auto);
    assert_eq!(choice.decoder, DecoderPreference::Automatic);
    assert_eq!(
        choice.notes,
        vec!["AV1: dieser Gastgeber hat keinen AV1-Kodierer – diese Sitzung: Automatisch"]
    );
    for served in [Auto, H264, Hevc, Sharp, Lossless] {
        let choice = session_choice(served, Some(&nvidia), &all);
        assert_eq!(
            (choice.codec, choice.notes.len()),
            (served, 0),
            "{served:?}"
        );
    }
    // Unknown host: the stored choice goes out; it depends on the host.
    assert_eq!(session_choice(Av1, None, &all).codec, Av1);
    assert!(host_matters(Av1, &all) && host_matters(Sharp, &all));
    assert!(!host_matters(Auto, &all) && !host_matters(Lossless, &all));

    // A host with the tiles only and „Dekodierer: Hardware“: the codec and
    // then the stage give way, each named.
    let tiles_only = cpu_host(false);
    let hardware = Viewer {
        decoder: DecoderPreference::Hardware,
        ..all
    };
    assert!(host_matters(Auto, &hardware));
    let choice = session_choice(Av1, Some(&tiles_only), &hardware);
    assert_eq!(
        (choice.codec, choice.decoder),
        (Auto, DecoderPreference::Automatic)
    );
    assert_eq!(choice.notes.len(), 2);
    assert!(choice.notes[1].starts_with("Dekodierer „Hardware“: kein Kodierer"));
    // A chosen library whose video the host cannot serve keeps the tiles.
    let ffmpeg = Viewer {
        library: DecoderLibrary::Ffmpeg,
        ..all
    };
    let choice = session_choice(Auto, Some(&tiles_only), &ffmpeg);
    assert_eq!(choice.library, DecoderLibrary::Automatic);
    assert!(choice.notes[0].starts_with("Dekodier-Bibliothek „FFmpeg“:"));
    assert_eq!(
        session_choice(Auto, Some(&nvidia), &ffmpeg).library,
        DecoderLibrary::Ffmpeg
    );
}

/// „Anzeige“: wgpu only in a wgpu window, Direct3D 11 only for FFmpeg's
/// D3D11VA pictures; „Automatisch“ always.
#[test]
fn codec_offer_task_display_entries_name_why_no_picture() {
    let facts = DisplayFacts {
        window: "wgpu",
        direct_error: None,
        library: DecoderLibrary::Automatic,
        decoder: DecoderPreference::Automatic,
        codec: CodecSetting::Auto,
        running: Some("FFmpeg h264 [D3D11VA]"),
    };
    for choice in DisplayChoice::ALL {
        assert_eq!(display_verdict(choice, &facts), Verdict::Usable);
    }
    let blocked =
        |facts: &DisplayFacts, choice| display_verdict(choice, facts).reason().map(str::to_string);
    let glow = DisplayFacts {
        window: "OpenGL (glow)",
        ..facts
    };
    assert_eq!(
        blocked(&glow, DisplayChoice::Wgpu).as_deref(),
        Some("das Fenster zeichnet mit OpenGL (glow); wgpu zeigt kein Bild")
    );
    let cases = [
        DisplayFacts {
            direct_error: Some("kein Adapter"),
            ..facts
        },
        DisplayFacts {
            library: DecoderLibrary::MediaFoundation,
            ..facts
        },
        DisplayFacts {
            decoder: DecoderPreference::Software,
            ..facts
        },
        DisplayFacts {
            codec: CodecSetting::Lossless,
            ..facts
        },
        DisplayFacts {
            running: Some("h264 [Software (Media Foundation)]"),
            ..facts
        },
    ];
    for case in cases {
        assert!(
            blocked(&case, DisplayChoice::Direct3d11).is_some(),
            "{case:?}"
        );
        assert_eq!(
            display_verdict(DisplayChoice::Automatic, &case),
            Verdict::Usable
        );
    }
}
