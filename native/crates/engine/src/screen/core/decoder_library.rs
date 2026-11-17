//! „Dekodierer-Bibliothek“ of the viewer (plan 2026-11-17 „Windows-Bild“
//! P8; the user's „lösch das andere nicht“): which library decodes on
//! Windows. `Automatic` is FFmpeg (D3D11VA, DXVA2, software) and Media
//! Foundation where FFmpeg does not open; `Ffmpeg` and `MediaFoundation`
//! take that library alone. A chosen library that this process does not
//! have at all leaves the other in place, so a choice never leaves the
//! window without a decoder. The choice of this process lives here like
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
/// offer and open under `choice`: the chosen one, both under `Automatic`,
/// and the other one where the chosen one has no decoder in this process
/// (`ffmpeg_present`: the FFmpeg DLLs loaded; `media_foundation_present`:
/// Media Foundation has a decoder transform).
pub fn library_allowed(
    library: DecoderLibrary,
    choice: DecoderLibrary,
    ffmpeg_present: bool,
    media_foundation_present: bool,
) -> bool {
    match (choice, library) {
        (DecoderLibrary::Automatic, _) => true,
        (chosen, own) if chosen == own => true,
        (DecoderLibrary::Ffmpeg, DecoderLibrary::MediaFoundation) => !ffmpeg_present,
        (DecoderLibrary::MediaFoundation, DecoderLibrary::Ffmpeg) => !media_foundation_present,
        _ => false,
    }
}

static DECODER_LIBRARY: AtomicU8 = AtomicU8::new(0);

/// Sets the library choice of this process (viewer start, a change).
pub fn set_decoder_library(library: DecoderLibrary) {
    DECODER_LIBRARY.store(library.code(), Ordering::Release);
}

/// The library choice of this process; automatic until one is set.
pub fn decoder_library() -> DecoderLibrary {
    match DECODER_LIBRARY.load(Ordering::Acquire) {
        1 => DecoderLibrary::Ffmpeg,
        2 => DecoderLibrary::MediaFoundation,
        _ => DecoderLibrary::Automatic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P8: FFmpeg and Media Foundation stay selectable; the default is
    /// automatic (FFmpeg first); a missing chosen library never leaves the
    /// viewer without a decoder (FFmpeg DLLs absent: Media Foundation).
    #[test]
    fn bildkette_task_ffmpeg_library_choice_and_fallback() {
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
        // DLLs absent: Media Foundation decodes whatever was chosen.
        assert!(library_allowed(MediaFoundation, Ffmpeg, false, true));
        // No Media Foundation (Windows N, Server): FFmpeg decodes.
        assert!(library_allowed(Ffmpeg, MediaFoundation, true, false));
        // The process value is not switched here: factories of parallel
        // tests read it.
        assert!(DecoderLibrary::ALL
            .iter()
            .all(|choice| [0, 1, 2].contains(&choice.code())));
    }
}
