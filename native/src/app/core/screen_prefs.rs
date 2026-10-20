//! Settings of the desktop viewer (SCR-M7, U25-6): „Niedrigste Latenz“,
//! „Schärfe vor Flüssigkeit“, „Verlustfrei“ and the fit (Einpassen/1:1) in
//! `<app data>/screen/viewer.json`. Read at the start (a missing or
//! unreadable file means the defaults, without a message), written
//! atomically at every change by the user. Full screen is not stored.
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const SCHEMA: u32 = 1;

/// How the picture is laid into the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Fit {
    /// As large as the window allows, with the aspect ratio kept.
    #[default]
    Fit,
    /// One picture pixel per screen pixel.
    Actual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewerPrefs {
    #[serde(default = "schema")]
    pub schema: u32,
    /// `AutoNoVsync` at the next start of the viewer (tearing possible).
    #[serde(default)]
    pub low_latency: bool,
    /// „Schärfe vor Flüssigkeit“.
    #[serde(default)]
    pub sharpness: bool,
    /// „Verlustfrei“ (the tile codec).
    #[serde(default)]
    pub lossless: bool,
    #[serde(default)]
    pub fit: Fit,
}

fn schema() -> u32 {
    SCHEMA
}

impl Default for ViewerPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            low_latency: false,
            sharpness: false,
            lossless: false,
            fit: Fit::Fit,
        }
    }
}

/// `<app data>/screen/viewer.json`.
pub(crate) fn prefs_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("viewer.json")
}

impl ViewerPrefs {
    /// The stored settings, or the defaults.
    pub(crate) fn load() -> Self {
        Self::load_from(&prefs_path())
    }

    pub(crate) fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        self.save_to(&prefs_path())
    }

    pub(crate) fn save_to(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        crate::support_dirs::write_private_atomic(path, &bytes)
    }
}
