//! FFmpeg in the codec register of the Windows viewer (plan 2026-11-17 P8):
//! one decoder factory per codec (H.264, HEVC, AV1) before the Media
//! Foundation decoders, and a gate around those, so „Dekodierer-Bibliothek“
//! (`core/decoder_library.rs`) picks FFmpeg, Media Foundation or both, with
//! the other one where the chosen one is absent. Nothing of FFmpeg loads
//! until a decoder factory is asked (host processes never are); then the
//! DLLs load once (`ffmpeg_ffi::ffmpeg`). A factory offers the hardware
//! stage where the viewer's GPU decodes the codec (`viewer_d3d11`) and the
//! software stage where FFmpeg has a software decoder of it, both 4:2:0 up
//! to `FrameSize::MAX`; „Dekodierer: Automatisch / Hardware / Software“
//! keeps the stages of its choice (`ffmpeg_plan::stages`). A hardware stage
//! that failed before its first picture, or whose device was lost, is not
//! tried again for that codec (P9: only that codec's stage; a lost device is
//! made anew by `viewer_d3d11::shared`, so the other codecs keep their
//! hardware) until the next viewer session opens (fix of 2026-11-18: it was
//! the rest of the process, and the main window's viewer runs for days),
//! and never so that a chosen „Dekodierer: Hardware“ is left without any
//! stage: then the given-up stages are tried again. The offers are kept once
//! the viewer device answered; a device that could not be made is asked
//! again at the next hello (before, one failure left the process without
//! hardware offers).
use std::io;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use super::ffmpeg_decoder::FfmpegDecoder;
use super::ffmpeg_ffi;
use crate::screen::core::codec_registry::DecoderFactory;
use crate::screen::core::decoder_health::{for_session, session_epoch};
use crate::screen::core::decoder_library::{decoder_library, library_allowed, DecoderLibrary};
use crate::screen::core::ffmpeg_plan::{self as plan, FfmpegStage};
use crate::screen::core::host_backend::ScreenDecoder;
use crate::screen::core::negotiate::{
    chosen_decoder_preference, decoder_preference, CodecKind, CodecOffer, DecoderPreference,
    H264Profile,
};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_binary::GopHeader;

/// Hardware stages given up, per codec, with the viewer session they were
/// given up in (`decoder_health::session_epoch`).
type GivenUpStages = (u64, Vec<(CodecKind, FfmpegStage)>);
static GIVEN_UP: Mutex<GivenUpStages> = Mutex::new((0, Vec::new()));

/// A hardware stage of `codec` failed; only that codec's stage is given up
/// (P9: a lost device no longer ends the hardware of every codec), until
/// the next viewer session opens.
pub(super) fn give_up(codec: CodecKind, stage: FfmpegStage) {
    let mut slot = GIVEN_UP.lock().unwrap_or_else(PoisonError::into_inner);
    let given_up = for_session(&mut *slot, session_epoch());
    if !given_up.contains(&(codec, stage)) {
        given_up.push((codec, stage));
        eprintln!(
            "FFmpeg: {} {} für diese Betrachter-Sitzung aufgegeben",
            codec.key(),
            stage.label()
        );
    }
}

fn hardware_allowed(codec: CodecKind, stage: FfmpegStage) -> bool {
    let mut slot = GIVEN_UP.lock().unwrap_or_else(PoisonError::into_inner);
    !for_session(&mut *slot, session_epoch()).contains(&(codec, stage))
}

/// The stages of `stages` to open: without the hardware stages given up,
/// unless that leaves a chosen „Dekodierer: Hardware“ (`chosen`) without
/// any stage (user rule 2026-11-18: a chosen setting is never replaced;
/// under „Automatisch“ the register's software pass follows instead).
fn open_stages(
    stages: Vec<FfmpegStage>,
    chosen: DecoderPreference,
    allowed: impl Fn(FfmpegStage) -> bool,
) -> Vec<FfmpegStage> {
    let left: Vec<FfmpegStage> = stages
        .iter()
        .copied()
        .filter(|stage| !stage.hardware() || allowed(*stage))
        .collect();
    if left.is_empty() && chosen == DecoderPreference::Hardware {
        stages
    } else {
        left
    }
}

/// Whether `library` may decode under this process's choice.
fn gate(library: DecoderLibrary, media_foundation_present: bool) -> bool {
    let choice = decoder_library();
    let ffmpeg_present =
        if (choice, library) == (DecoderLibrary::Ffmpeg, DecoderLibrary::MediaFoundation) {
            ffmpeg_ffi::present()
        } else {
            true
        };
    library_allowed(library, choice, ffmpeg_present, media_foundation_present)
}

fn not_chosen(library: DecoderLibrary) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{} is not the chosen decoder library", library.label()),
    )
}

struct FfmpegFactory {
    codec: CodecKind,
    media_foundation_present: bool,
    offers: OnceLock<Vec<CodecOffer>>,
}

impl FfmpegFactory {
    fn stage_offers(&self, hardware: bool) -> Vec<CodecOffer> {
        if self.codec == CodecKind::H264 {
            [H264Profile::High, H264Profile::ConstrainedBaseline]
                .into_iter()
                .map(|profile| CodecOffer::h264(profile, hardware).with_max(FrameSize::MAX))
                .collect()
        } else {
            vec![CodecOffer::video(self.codec, false, hardware).with_max(FrameSize::MAX)]
        }
    }

    /// The offers and whether they are final: without the viewer device
    /// the hardware stage is unknown, not absent, and is asked again.
    fn find_offers(&self, ffmpeg: &ffmpeg_ffi::Ffmpeg) -> (Vec<CodecOffer>, bool) {
        let device = super::viewer_d3d11::shared();
        let known = device.is_ok();
        let hardware = device
            .as_ref()
            .is_ok_and(|device| device.decodes(self.codec, None));
        let software = plan::decoder_name(self.codec, FfmpegStage::Software)
            .and_then(|name| std::ffi::CString::new(name).ok())
            // SAFETY: a name lookup in the loaded DLL.
            .is_some_and(|name| unsafe { !(ffmpeg.find_decoder_by_name)(name.as_ptr()).is_null() });
        let mut offers = Vec::new();
        if hardware {
            offers.extend(self.stage_offers(true));
        }
        if software {
            offers.extend(self.stage_offers(false));
        }
        (offers, known)
    }
}

impl DecoderFactory for FfmpegFactory {
    fn name(&self) -> String {
        plan::factory_name(self.codec)
    }

    fn offers(&self) -> Vec<CodecOffer> {
        if !gate(DecoderLibrary::Ffmpeg, self.media_foundation_present) {
            return Vec::new();
        }
        if let Some(offers) = self.offers.get() {
            return offers.clone();
        }
        // Not kept while FFmpeg is missing: a copy installed later counts.
        // Not kept while the viewer device is missing either.
        match ffmpeg_ffi::ffmpeg() {
            Ok(ffmpeg) => match self.find_offers(&ffmpeg) {
                (offers, true) => self.offers.get_or_init(|| offers).clone(),
                (offers, false) => offers,
            },
            Err(_) => Vec::new(),
        }
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if !gate(DecoderLibrary::Ffmpeg, self.media_foundation_present) {
            return Err(not_chosen(DecoderLibrary::Ffmpeg));
        }
        if header.codec != self.codec {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{} does not decode {}", self.name(), header.codec.key()),
            ));
        }
        let ffmpeg = ffmpeg_ffi::ffmpeg()?;
        let fits = super::viewer_d3d11::shared()
            .is_ok_and(|device| device.decodes(header.codec, Some(header.size)));
        let stages = open_stages(
            plan::stages(decoder_preference(), header.codec, fits, true),
            chosen_decoder_preference(),
            |stage| hardware_allowed(header.codec, stage),
        );
        let mut refusals = Vec::new();
        for stage in stages {
            match FfmpegDecoder::open(ffmpeg.clone(), header, stage) {
                Ok(decoder) => return Ok(Box::new(decoder)),
                Err(error) => refusals.push(format!("{}: {error}", stage.label())),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{}: {}", self.name(), refusals.join("; ")),
        ))
    }

    fn keeps_references(&self, codec: CodecKind) -> bool {
        codec == self.codec && codec != CodecKind::H264
    }
}

/// A Media Foundation decoder behind „Dekodierer-Bibliothek“.
struct MediaFoundationGate {
    inner: Arc<dyn DecoderFactory>,
}

impl DecoderFactory for MediaFoundationGate {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn offers(&self) -> Vec<CodecOffer> {
        if gate(DecoderLibrary::MediaFoundation, true) {
            self.inner.offers()
        } else {
            Vec::new()
        }
    }

    fn open(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if !gate(DecoderLibrary::MediaFoundation, true) {
            return Err(not_chosen(DecoderLibrary::MediaFoundation));
        }
        self.inner.open(header)
    }

    fn keeps_references(&self, codec: CodecKind) -> bool {
        self.inner.keeps_references(codec)
    }

    /// The adapter of the wrapped decoder (P3: the key of its verdicts).
    fn device(&self) -> String {
        self.inner.device()
    }
}

/// The decoders of the Windows viewer: FFmpeg's three factories first, then
/// the Media Foundation decoders `media_foundation` behind the gate.
pub(super) fn decoders(
    media_foundation: Vec<Arc<dyn DecoderFactory>>,
) -> Vec<Arc<dyn DecoderFactory>> {
    let media_foundation_present = !media_foundation.is_empty();
    let mut decoders: Vec<Arc<dyn DecoderFactory>> =
        [CodecKind::H264, CodecKind::Hevc, CodecKind::Av1]
            .into_iter()
            .map(|codec| {
                Arc::new(FfmpegFactory {
                    codec,
                    media_foundation_present,
                    offers: OnceLock::new(),
                }) as Arc<dyn DecoderFactory>
            })
            .collect();
    decoders.extend(
        media_foundation
            .into_iter()
            .map(|inner| Arc::new(MediaFoundationGate { inner }) as Arc<dyn DecoderFactory>),
    );
    decoders
}

#[cfg(test)]
#[path = "ffmpeg_codecs_tests.rs"]
mod tests;
