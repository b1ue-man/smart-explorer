//! Plan 2026-11-17 P9: „Dekodierer: Automatisch“ opens every hardware
//! decoder of the register before any software decoder (FFmpeg D3D11VA,
//! then Media Foundation hardware, then software), and a factory sees the
//! pass it runs in as the process's choice.
use std::io;
use std::sync::{Arc, Mutex};

use crate::screen::core::codec_registry::{CodecRegistry, DecoderFactory};
use crate::screen::core::host_backend::{ScreenDecoder, TilesScreenDecoder};
use crate::screen::core::negotiate::{
    decoder_preference, with_decoder_pass, CodecKind, CodecOffer, DecoderPreference,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::{GopHeader, StreamKind};

const SIZE: FrameSize = FrameSize::new(64, 32);

/// What each factory saw when the register asked it to open.
type Seen = Arc<Mutex<Vec<(&'static str, DecoderPreference)>>>;

/// A library with stages: it offers hardware and/or software and opens
/// only in the stages `opens` names (a given-up hardware stage refuses).
struct Library {
    name: &'static str,
    offers_hardware: bool,
    offers_software: bool,
    opens: &'static [DecoderPreference],
    seen: Seen,
}

impl DecoderFactory for Library {
    fn name(&self) -> String {
        self.name.into()
    }
    fn offers(&self) -> Vec<CodecOffer> {
        let mut offers = Vec::new();
        if self.offers_hardware {
            offers.push(CodecOffer::video(CodecKind::Av1, false, true));
        }
        if self.offers_software {
            offers.push(CodecOffer::video(CodecKind::Av1, false, false));
        }
        offers
    }
    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        let pass = decoder_preference();
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((self.name, pass));
        if self.opens.contains(&pass) {
            Ok(Box::new(TilesScreenDecoder::new(header.size)?))
        } else {
            Err(io::Error::other("stage given up"))
        }
    }
}

fn header() -> GopHeader {
    GopHeader {
        gop_id: 1,
        frame_contract: None,
        family_layout_revision: None,
        kind: StreamKind::Video,
        codec: CodecKind::Av1,
        profile: None,
        size: SIZE,
    }
}

/// FFmpeg (hardware given up, software opens) before Media Foundation
/// (hardware opens), as in the register of the Windows viewer.
fn register(seen: &Seen) -> CodecRegistry {
    use DecoderPreference::*;
    CodecRegistry::new(
        Vec::new(),
        vec![
            Arc::new(Library {
                name: "ffmpeg",
                offers_hardware: true,
                offers_software: true,
                opens: &[Software],
                seen: seen.clone(),
            }) as Arc<dyn DecoderFactory>,
            Arc::new(Library {
                name: "media_foundation",
                offers_hardware: true,
                offers_software: true,
                opens: &[Hardware, Software],
                seen: seen.clone(),
            }),
        ],
    )
}

fn opened_with(choice: DecoderPreference) -> Vec<(&'static str, DecoderPreference)> {
    let seen: Seen = Arc::default();
    let registry = register(&seen);
    let opened = with_decoder_pass(choice, || {
        let opened = registry.open_decoder(&header());
        // The passes are over: the choice of this thread is back.
        assert_eq!(decoder_preference(), choice);
        opened
    });
    assert!(opened.is_ok(), "{choice:?}: {:?}", opened.err());
    let mut calls = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::take(&mut *calls)
}

#[test]
fn p9_task_automatic_tries_every_hardware_decoder_before_software() {
    use DecoderPreference::*;
    assert_eq!(Automatic.passes(), &[Hardware, Software]);
    assert_eq!(Hardware.passes(), &[Hardware]);
    assert_eq!(Software.passes(), &[Software]);
    // Automatic: FFmpeg's hardware refuses, Media Foundation's hardware
    // opens; FFmpeg's software is never asked.
    assert_eq!(
        opened_with(Automatic),
        vec![("ffmpeg", Hardware), ("media_foundation", Hardware)]
    );
    // Hardware: the same, nothing else.
    assert_eq!(
        opened_with(Hardware),
        vec![("ffmpeg", Hardware), ("media_foundation", Hardware)]
    );
    // Software: the first library's software stage.
    assert_eq!(opened_with(Software), vec![("ffmpeg", Software)]);
}

#[test]
fn p9_task_automatic_falls_back_to_software_only_after_all_hardware() {
    use DecoderPreference::*;
    let seen: Seen = Arc::default();
    let registry = CodecRegistry::new(
        Vec::new(),
        vec![
            Arc::new(Library {
                name: "ffmpeg",
                offers_hardware: true,
                offers_software: true,
                opens: &[Software],
                seen: seen.clone(),
            }) as Arc<dyn DecoderFactory>,
            Arc::new(Library {
                name: "media_foundation",
                offers_hardware: true,
                offers_software: false,
                opens: &[],
                seen: seen.clone(),
            }),
        ],
    );
    let opened = with_decoder_pass(Automatic, || registry.open_decoder(&header()));
    assert!(opened.is_ok());
    assert_eq!(
        *seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            ("ffmpeg", Hardware),
            ("media_foundation", Hardware),
            ("ffmpeg", Software)
        ]
    );
    // With no stage that opens, the error names every refusal and pass.
    let none = CodecRegistry::new(
        Vec::new(),
        vec![Arc::new(Library {
            name: "none",
            offers_hardware: true,
            offers_software: true,
            opens: &[],
            seen,
        }) as Arc<dyn DecoderFactory>],
    );
    let error = with_decoder_pass(Automatic, || none.open_decoder(&header()))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(error.contains("none (hardware)"), "{error}");
    assert!(error.contains("none (software)"), "{error}");
}

/// P9 Zwischenmodus: „Scharf (GPU)“ is a codec choice between AV1 and
/// „Verlustfrei“; it negotiates H.264 and HEVC (never AV1) with the tiles
/// as the last fallback and sends `SET_MODE` `sharpness`.
#[test]
fn p9_task_sharp_codec_setting() {
    use crate::screen::core::negotiate::{CodecPreference, CodecSetting};
    let sharp = CodecSetting::Sharp;
    assert_eq!(CodecSetting::parse("sharp"), Some(sharp));
    assert_eq!(sharp.key(), "sharp");
    assert_eq!(sharp.label(), "Scharf (GPU)");
    assert_eq!(CodecSetting::ALL[4], sharp);
    assert_eq!(CodecSetting::ALL[5], CodecSetting::Lossless);
    assert!(sharp.sharpness() && !sharp.lossless());
    assert!(!CodecSetting::Auto.sharpness() && !CodecSetting::Lossless.sharpness());
    let preference = sharp.preference();
    assert_eq!(preference, CodecPreference::Sharp);
    assert_eq!(CodecPreference::parse("sharp"), Some(preference));
    assert_eq!(preference.key(), "sharp");
    assert_eq!(preference.codec(), None);
    assert!(preference.admits(CodecKind::H264) && preference.admits(CodecKind::Hevc));
    assert!(preference.admits(CodecKind::Tiles) && !preference.admits(CodecKind::Av1));
    let offers = [
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::tiles(),
    ];
    let kept: Vec<CodecKind> = preference
        .filter_decoders(&offers)
        .iter()
        .map(|offer| offer.codec)
        .collect();
    // Plan 2026-11-18 „Netzrate“: a chosen mode offers no tiles.
    assert_eq!(kept, [CodecKind::Hevc]);
}
