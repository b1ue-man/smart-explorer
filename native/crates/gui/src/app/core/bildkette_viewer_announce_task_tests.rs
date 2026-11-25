//! The announcement of `gop_continue` (plan 2026-11-14 „Bildkette“ B-36,
//! V1): only when every decoder that may open for the request's offers
//! keeps its references over a continuation.
use std::io;
use std::sync::Arc;

use super::announces_with;
use crate::daemon::StreamRequest;
use crate::screen::core::codec_registry::{CodecRegistry, DecoderFactory};
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::negotiate::{
    CodecKind, CodecOffer, CodecSetting, DecoderPreference, H264Profile,
};
use crate::screen::core::wire_binary::GopHeader;

/// A decoder factory of one codec and stage.
struct Factory {
    offers: Vec<CodecOffer>,
    keeps: bool,
}

impl DecoderFactory for Factory {
    fn name(&self) -> String {
        "factory".into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        self.offers.clone()
    }
    fn open(&self, _header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        Err(io::Error::other("not opened in this test"))
    }
    fn keeps_references(&self, codec: CodecKind) -> bool {
        self.keeps && self.offers.iter().any(|offer| offer.codec == codec)
    }
}

fn registry(factories: Vec<Factory>) -> CodecRegistry {
    CodecRegistry::new(
        Vec::new(),
        factories
            .into_iter()
            .map(|factory| Arc::new(factory) as Arc<dyn DecoderFactory>)
            .collect(),
    )
}

#[test]
fn bildkette_task_viewer_announces_only_with_capable_decoders() {
    // Windows-like: H.264 decoders cannot continue, HEVC can.
    let windows = registry(vec![
        Factory {
            offers: vec![CodecOffer::h264(H264Profile::High, true)],
            keeps: false,
        },
        Factory {
            offers: vec![CodecOffer::video(CodecKind::Hevc, false, true)],
            keeps: true,
        },
    ]);
    let request = |codec: CodecSetting, decoder: DecoderPreference| StreamRequest {
        expected_node: None,
        mode: crate::screen::core::wire_hello::SessionMode::View,
        source: None,
        decoders: windows.decoder_offers(),
        display_size: None,
        frame_capabilities: None,
        max_fps: 60,
        max_rate_bps: None,
        codec: Some(codec),
        decoder: Some(decoder),
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
        delivery: None,
    };
    let auto = DecoderPreference::Automatic;
    // H.264 among the offers: a decoder that may open cannot continue.
    assert!(!announces_with(
        &windows,
        &request(CodecSetting::Auto, auto)
    ));
    assert!(!announces_with(
        &windows,
        &request(CodecSetting::H264, auto)
    ));
    assert!(!announces_with(
        &windows,
        &request(CodecSetting::Lossless, auto)
    ));
    // Only HEVC: every decoder that may open continues.
    assert!(announces_with(&windows, &request(CodecSetting::Hevc, auto)));
    // HEVC in hardware only, software chosen: only the tiles remain.
    let software = request(CodecSetting::Hevc, DecoderPreference::Software);
    assert!(!announces_with(&windows, &software));
    // A capable decoder with an incapable fallback for the same codec.
    let mixed = registry(vec![
        Factory {
            offers: vec![CodecOffer::video(CodecKind::Hevc, false, true)],
            keeps: true,
        },
        Factory {
            offers: vec![CodecOffer::video(CodecKind::Hevc, false, false)],
            keeps: false,
        },
    ]);
    let offers = mixed.decoder_offers();
    assert!(!mixed.keeps_references(&offers, auto));
    assert!(mixed.keeps_references(&offers, DecoderPreference::Hardware));
    // No registry (tests, fakes) and only tiles announce nothing.
    assert!(!registry(Vec::new()).keeps_references(&[CodecOffer::tiles()], auto));
}

#[test]
fn bildkette_task_viewer_decoder_switch_decision_without_reopening() {
    use crate::app::screen_modules::screen_viewer_fake::{open_preview, Fake};
    use crate::screen::core::pixel_convert::FrameSize;
    use crate::screen::core::wire_control::StreamInfo;
    use crate::share::access_grant::CapabilityDecision;
    use std::sync::atomic::Ordering;

    // H.264 in both stages, HEVC in hardware only.
    let both = registry(vec![
        Factory {
            offers: vec![CodecOffer::h264(H264Profile::High, true)],
            keeps: false,
        },
        Factory {
            offers: vec![
                CodecOffer::h264(H264Profile::High, false),
                CodecOffer::video(CodecKind::Hevc, false, true),
            ],
            keeps: false,
        },
    ]);
    let fake = Fake::new(CapabilityDecision::Allow);
    let backend = fake.backend();
    let session = open_preview(&fake, &backend);
    let switches = || session.shared.local.switches.load(Ordering::Acquire);
    let stream = |codec, profile| StreamInfo {
        codec,
        profile,
        size: FrameSize::new(1920, 1080),
        encoder: "probe".into(),
        hardware: true,
        decoder_hardware: Some(true),
        source_size: None,
    };
    // No stream yet: nothing runs that could take the choice over.
    assert!(!session.switch_decoder_with(&both, DecoderPreference::Software));
    // H.264 has a software decoder: taken over, no new session.
    let h264 = stream(CodecKind::H264, Some(H264Profile::High));
    session.shared.update(|info| info.stream = Some(h264));
    assert!(session.switch_decoder_with(&both, DecoderPreference::Software));
    assert_eq!(switches(), 1);
    // HEVC has none: the session opens again (the caller's `Transition`).
    session
        .shared
        .update(|info| info.stream = Some(stream(CodecKind::Hevc, None)));
    assert!(!session.switch_decoder_with(&both, DecoderPreference::Software));
    assert!(session.switch_decoder_with(&both, DecoderPreference::Hardware));
    assert_eq!(switches(), 2);
    // Tiles: no video decoder runs; the next one opens under the choice.
    session
        .shared
        .update(|info| info.stream = Some(stream(CodecKind::Tiles, None)));
    assert!(session.switch_decoder_with(&both, DecoderPreference::Software));
    assert_eq!(switches(), 2);
    assert_eq!(fake.opens(), 1, "the switch opened no second connection");
}
