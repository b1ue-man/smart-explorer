//! What a tile of the group view shows (SCR-M7, U25-10): the state of the
//! device and of its preview, the text on the tile and the color of the dot
//! beside the name (green: a picture, yellow: waiting, red: an error, gray:
//! nothing to show).
use eframe::egui::Color32;

use super::screen_texts::error_text;

/// What a tile shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    Offline,
    Probing,
    ProbeFailed(String),
    Unsupported,
    NotShared,
    Ask,
    Waiting(String),
    Picture,
    NoAnswer,
    Failed(String),
    Limit,
}

impl Face {
    pub(crate) fn dot(&self) -> Color32 {
        match self {
            Self::Picture => Color32::from_rgb(70, 190, 100),
            Self::Probing | Self::Ask | Self::Waiting(_) | Self::NoAnswer => {
                Color32::from_rgb(230, 190, 60)
            }
            Self::ProbeFailed(_) | Self::Failed(_) | Self::Limit => Color32::from_rgb(220, 80, 70),
            Self::Offline | Self::Unsupported | Self::NotShared => Color32::from_gray(120),
        }
    }

    pub(crate) fn text(&self) -> String {
        match self {
            Self::Offline => "Offline".into(),
            Self::Probing => "Prüfe …".into(),
            Self::ProbeFailed(error) => format!("Nicht erreichbar: {error}"),
            Self::Unsupported => "Keine Bildschirmfreigabe (ältere Version)".into(),
            Self::NotShared => "Bildschirm nicht freigegeben".into(),
            Self::Ask => "Ansehen anfragen".into(),
            Self::Waiting(text) | Self::Failed(text) => text.clone(),
            Self::Picture => String::new(),
            Self::NoAnswer => "Keine Antwort".into(),
            Self::Limit => error_text("session_limit", None),
        }
    }
}
