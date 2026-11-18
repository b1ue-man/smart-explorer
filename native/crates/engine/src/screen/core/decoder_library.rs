//! „Dekodierer-Bibliothek“ of the viewer (plan 2026-11-17 „Windows-Bild“
//! P8; the user's „lösch das andere nicht“): which library decodes on
//! Windows. `Automatic` is FFmpeg (D3D11VA, DXVA2, software) and Media
//! Foundation where FFmpeg does not open; `Ffmpeg` and `MediaFoundation`
//! take that library alone. User rule of 2026-11-18 („jede Einstellung
//! wird genau so verwendet“): a chosen library that this process does not
//! have is not replaced by the other; the gate notes it (`chosen_missing`)
//! and the viewer shows „<Bibliothek> nicht vorhanden“. Only „Automatisch“
//! takes whichever library is there. The choice of this process lives here like
//! „Dekodierer“ (`negotiate::set_decoder_preference`): the desktop viewer
//! and `se screen view` set it, the decoder factories read it when they
//! offer and open. Elsewhere (Linux, Android) there is one library and the
//! choice changes nothing.
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// A decoder library of Windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoderLibrary {
    /// FFmpeg first, Media Foundation when FFmpeg fails.
    #[default]
    #[serde(rename = "auto")]
    Automatic,
    Ffmpeg,
    MediaFoundation,
}

impl DecoderLibrary {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Ffmpeg, Self::MediaFoundation];

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.key() == text)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Ffmpeg => "ffmpeg",
            Self::MediaFoundation => "media_foundation",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatisch",
            Self::Ffmpeg => "FFmpeg",
            Self::MediaFoundation => "Media Foundation",
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Automatic => 0,
            Self::Ffmpeg => 1,
            Self::MediaFoundation => 2,
        }
    }
}

/// Whether the decoders of `library` (`Ffmpeg` or `MediaFoundation`) may
/// offer and open under `choice`: the chosen one alone, both under
/// `Automatic`. The other library never stands in for a chosen one; when
/// the facts show the chosen one absent (`ffmpeg_present`: the FFmpeg DLLs
/// loaded; `media_foundation_present`: Media Foundation has a decoder
/// transform), that is noted for the viewer's message (`chosen_missing`).
pub fn library_allowed(
    library: DecoderLibrary,
    choice: DecoderLibrary,
    ffmpeg_present: bool,
    media_foundation_present: bool,
) -> bool {
    match (choice, library) {
        (DecoderLibrary::Automatic, _) => true,
        (chosen, own) if chosen == own => true,
        (DecoderLibrary::Ffmpeg, DecoderLibrary::MediaFoundation) => {
            note_presence(DecoderLibrary::Ffmpeg, ffmpeg_present);
            false
        }
        (DecoderLibrary::MediaFoundation, DecoderLibrary::Ffmpeg) => {
            note_presence(DecoderLibrary::MediaFoundation, media_foundation_present);
            false
        }
        _ => false,
    }
}

/// The library the gate last found absent while it was chosen (code, 0: none).
static MISSING: AtomicU8 = AtomicU8::new(0);

fn note_presence(chosen: DecoderLibrary, present: bool) {
    let code = chosen.code();
    if present {
        let _ = MISSING.compare_exchange(code, 0, Ordering::AcqRel, Ordering::Acquire);
    } else {
        MISSING.store(code, Ordering::Release);
    }
}

fn of_code(code: u8) -> DecoderLibrary {
    match code {
        1 => DecoderLibrary::Ffmpeg,
        2 => DecoderLibrary::MediaFoundation,
        _ => DecoderLibrary::Automatic,
    }
}

/// The chosen library when the gate found it absent in this process; the
/// viewer shows „<Bibliothek> nicht vorhanden“. `None` under „Automatisch“
/// and while the chosen one is there (or was not checked yet).
pub fn chosen_missing() -> Option<DecoderLibrary> {
    let missing = of_code(MISSING.load(Ordering::Acquire));
    (missing != DecoderLibrary::Automatic && missing == decoder_library()).then_some(missing)
}

/// The viewer's text for a chosen library that is absent.
pub fn missing_text(library: DecoderLibrary) -> String {
    format!(
        "Dekodier-Bibliothek „{}“ nicht vorhanden; sie wird nicht durch eine andere ersetzt \
         (nur „Automatisch“ weicht aus)",
        library.label()
    )
}

static DECODER_LIBRARY: AtomicU8 = AtomicU8::new(0);

/// Sets the library choice of this process (viewer start, a change).
pub fn set_decoder_library(library: DecoderLibrary) {
    DECODER_LIBRARY.store(library.code(), Ordering::Release);
}

/// The library choice of this process; automatic until one is set.
pub fn decoder_library() -> DecoderLibrary {
    of_code(DECODER_LIBRARY.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P8 and the user rule of 2026-11-18: FFmpeg and Media Foundation stay
    /// selectable, the default is automatic (FFmpeg first); a chosen
    /// library is never replaced by the other, also when it is absent.
    #[test]
    fn wahlgilt_task_library_choice_never_takes_the_other() {
        use DecoderLibrary::*;
        assert_eq!(DecoderLibrary::default(), Automatic);
        for choice in DecoderLibrary::ALL {
            assert_eq!(DecoderLibrary::parse(choice.key()), Some(choice));
        }
        assert_eq!(DecoderLibrary::parse("mf"), None);
        assert_eq!(
            serde_json::to_value(MediaFoundation).unwrap(),
            "media_foundation"
        );
        assert_eq!(serde_json::to_value(Automatic).unwrap(), "auto");
        // Automatic: both, FFmpeg's factories stand first in the register.
        assert!(library_allowed(Ffmpeg, Automatic, true, true));
        assert!(library_allowed(MediaFoundation, Automatic, true, true));
        assert!(library_allowed(Ffmpeg, Automatic, false, true));
        // A chosen library alone while it is present.
        assert!(library_allowed(Ffmpeg, Ffmpeg, true, true));
        assert!(!library_allowed(MediaFoundation, Ffmpeg, true, true));
        assert!(library_allowed(
            MediaFoundation,
            MediaFoundation,
            true,
            true
        ));
        assert!(!library_allowed(Ffmpeg, MediaFoundation, true, true));
        // Chosen and absent: the other one does not stand in.
        assert!(!library_allowed(MediaFoundation, Ffmpeg, false, true));
        assert!(!library_allowed(Ffmpeg, MediaFoundation, true, false));
        assert_eq!(of_code(Ffmpeg.code()), Ffmpeg);
        assert_eq!(of_code(MediaFoundation.code()), MediaFoundation);
        assert_eq!(
            missing_text(Ffmpeg),
            "Dekodier-Bibliothek „FFmpeg“ nicht vorhanden; sie wird nicht durch eine andere \
             ersetzt (nur „Automatisch“ weicht aus)"
        );
        // The process value is not switched here: factories of parallel
        // tests read it.
        assert!(DecoderLibrary::ALL
            .iter()
            .all(|choice| [0, 1, 2].contains(&choice.code())));
    }
}
