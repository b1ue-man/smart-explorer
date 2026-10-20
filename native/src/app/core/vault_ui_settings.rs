//! The one setting of the vault windows that is not a vault's own setting
//! (U29-7, hand-over of U20): „Beim Öffnen einhängen“, stored for all vault
//! sites in `<app data>/vaults/ui.json` as `{"schema":1,"mount_on_open":true}`.
//! A missing or unreadable file means the default, `true` (C7.2); writing is
//! atomic and private, and keys of a newer version stay in the file.
#![allow(dead_code)] // some accessors are the acceptance signals (tests) and the surfaces of later packages
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// File name inside the vault list folder (it holds only `*.conf` sites and
/// this file; the vault list reads `*.conf` only).
const FILE_NAME: &str = "ui.json";
/// The file is a few bytes of JSON; a larger file is not ours.
const MAX_BYTES: u64 = 64 * 1024;
const SCHEMA: u64 = 1;

/// The settings of the vault windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VaultUiSettings {
    /// Open vault files in the vault's drive, mounting it when needed.
    pub(crate) mount_on_open: bool,
}

impl Default for VaultUiSettings {
    fn default() -> Self {
        Self {
            mount_on_open: true,
        }
    }
}

/// The settings a file body says; anything unreadable is the default.
pub(crate) fn parse(text: &str) -> VaultUiSettings {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    VaultUiSettings {
        mount_on_open: value
            .get("mount_on_open")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }
}

/// `<app data>/vaults/ui.json`.
pub(crate) fn settings_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join(crate::vault::VAULTS_DIR)
        .join(FILE_NAME)
}

/// The settings stored at `path`.
pub(crate) fn load_from(path: &Path) -> VaultUiSettings {
    crate::support_dirs::read_private_text(path, MAX_BYTES)
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Stores `settings` at `path`, keeping unknown keys of the file.
pub(crate) fn save_to(path: &Path, settings: &VaultUiSettings) -> io::Result<()> {
    let mut object: Map<String, Value> = crate::support_dirs::read_private_text(path, MAX_BYTES)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_default();
    object.insert("schema".into(), json!(SCHEMA));
    object.insert("mount_on_open".into(), json!(settings.mount_on_open));
    let body = serde_json::to_vec_pretty(&Value::Object(object)).map_err(io::Error::other)?;
    crate::support_dirs::write_private_atomic(path, &body)
}

/// „Beim Öffnen einhängen“ (replaces the constant `MOUNT_ON_OPEN`).
pub(crate) fn mount_on_open() -> bool {
    load_from(&settings_path()).mount_on_open
}

/// Switches „Beim Öffnen einhängen“.
pub(crate) fn set_mount_on_open(on: bool) -> io::Result<()> {
    save_to(&settings_path(), &VaultUiSettings { mount_on_open: on })
}
