//! Acceptance of plan 2026-11-14 „Bildkette“ BK6, item 7 and 4.10: the
//! codec register as `se screen codec --json` shows it and the decoder
//! offers under „Dekodierer“.
use super::{high, hw, sw};
use crate::screen::core::negotiate::{CodecKind, CodecOffer};

/// An encoder or decoder of the given offers that never opens.
struct Fake(&'static str, Vec<CodecOffer>);

impl crate::screen::core::codec_registry::EncoderFactory for Fake {
    fn name(&self) -> String {
        self.0.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        self.1.clone()
    }
    fn open(
        &self,
        _choice: &crate::screen::core::negotiate::CodecChoice,
        _size: crate::screen::core::pixel_convert::FrameSize,
        _fps: u32,
    ) -> std::io::Result<Box<dyn crate::screen::core::host_backend::ScreenEncoder>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

impl crate::screen::core::codec_registry::DecoderFactory for Fake {
    fn name(&self) -> String {
        self.0.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        self.1.clone()
    }
    fn open(
        &self,
        _header: &crate::screen::core::wire_binary::GopHeader,
    ) -> std::io::Result<Box<dyn crate::screen::core::host_backend::ScreenDecoder>> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Item 7 of BK6: `se screen codec --json` names per codec its encoders
/// and decoders with `hardware`, the profiles and refused settings, the
/// fields of v0.5.196 unchanged; the offers keep H.264 and the tiles in
/// front (the legacy list) and HEVC/AV1 after; „Dekodierer“ keeps the
/// stages of its kind (4.10).
#[test]
fn bildkette_task_codec_register_report_and_decoder_choice() {
    use crate::screen::core::codec_registry::{
        CodecRegistry, DecoderFactory, EncoderFactory, OpenH264Report, OpenH264State,
    };
    use crate::screen::core::negotiate::DecoderPreference;
    use std::sync::Arc;
    let gpu = Fake(
        "gpu",
        vec![
            hw(CodecKind::Hevc),
            CodecOffer::video(CodecKind::Hevc, true, true),
            high(true),
        ],
    );
    let both = Fake("both", vec![hw(CodecKind::Av1), sw(CodecKind::Av1)]);
    let encoders: Vec<Arc<dyn EncoderFactory>> = vec![Arc::new(gpu)];
    let decoders: Vec<Arc<dyn DecoderFactory>> = vec![Arc::new(both)];
    let registry = CodecRegistry::new(encoders, decoders);
    let report = registry.report(OpenH264Report::new(OpenH264State::Absent, None));
    let json = serde_json::to_value(&report).unwrap();
    let rows = json["encoders"].as_array().unwrap();
    assert_eq!(rows[0]["codec"], "hevc");
    assert_eq!(rows[0]["hardware"], true);
    assert_eq!(rows[0]["profiles"], serde_json::json!(["high"]));
    assert_eq!(
        rows[0]["profile_names"],
        serde_json::json!(["main", "main_444"])
    );
    assert!(rows[0].get("refused_settings").is_none());
    assert_eq!(rows[1]["codec"], "h264");
    assert_eq!(rows[1]["profile_names"], serde_json::json!(["high"]));
    assert_eq!(
        rows.last().unwrap(),
        &serde_json::json!({"codec": "tiles", "profiles": [], "name": "tiles", "hardware": false})
    );
    let offers = registry.encoder_offers();
    let kinds: Vec<CodecKind> = offers.iter().map(|offer| offer.codec).collect();
    assert_eq!(
        kinds,
        vec![
            CodecKind::H264,
            CodecKind::Tiles,
            CodecKind::Hevc,
            CodecKind::Hevc
        ]
    );
    // „Dekodierer“: hardware keeps the hardware stage, software the other.
    let viewer = registry.decoder_offers();
    let hardware = DecoderPreference::Hardware.filter_offers(&viewer);
    let software = DecoderPreference::Software.filter_offers(&viewer);
    assert!(hardware
        .iter()
        .all(|o| o.hardware || o.codec == CodecKind::Tiles));
    assert!(software.iter().all(|o| !o.hardware));
    assert_eq!(hardware.len(), 2);
    assert_eq!(software.len(), 2);
    assert_eq!(DecoderPreference::Automatic.filter_offers(&viewer), viewer);
}
