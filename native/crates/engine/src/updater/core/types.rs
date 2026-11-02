use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerifiedPayload {
    path: PathBuf,
    sha256: String,
}

impl VerifiedPayload {
    pub(super) fn new(path: PathBuf, sha256: String) -> Result<Self, String> {
        let sha256 = sha256.trim().to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err("Ungueltige SHA-256 fuer gestagte Update-Datei".to_string());
        }
        Ok(Self { path, sha256 })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StagedUpdate {
    schema: u32,
    version: String,
    app: VerifiedPayload,
    helper: VerifiedPayload,
    cli: VerifiedPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launcher: Option<VerifiedPayload>,
}

impl StagedUpdate {
    pub(super) fn new(
        version: String,
        app: VerifiedPayload,
        helper: VerifiedPayload,
        cli: VerifiedPayload,
        launcher: Option<VerifiedPayload>,
    ) -> Result<Self, String> {
        if version.trim().is_empty() {
            return Err("Update-Version ist leer".to_string());
        }
        Ok(Self {
            schema: if launcher.is_some() { 2 } else { 1 },
            version,
            app,
            helper,
            cli,
            launcher,
        })
    }

    pub(super) fn validate_schema(&self) -> Result<(), String> {
        if !matches!(self.schema, 1 | 2) || (self.schema == 2) != self.launcher.is_some() {
            return Err(format!(
                "Nicht unterstuetztes Staging-Manifest-Schema {}",
                self.schema
            ));
        }
        if self.version.trim().is_empty()
            || !self
                .version
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        {
            return Err("Gestagtes Update enthaelt eine ungueltige Version".to_string());
        }
        if super::core::parse_ver(&self.version) > (0, 5, 184) && self.launcher.is_none() {
            return Err(
                "Diese Update-Version benoetigt einen SHA-256-geprueften MCP-Starter".into(),
            );
        }
        Ok(())
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn app(&self) -> &VerifiedPayload {
        &self.app
    }

    pub fn helper(&self) -> &VerifiedPayload {
        &self.helper
    }

    pub fn cli(&self) -> &VerifiedPayload {
        &self.cli
    }

    pub fn launcher(&self) -> Option<&VerifiedPayload> {
        self.launcher.as_ref()
    }

    pub(super) fn payloads(&self) -> Vec<&VerifiedPayload> {
        let mut payloads = vec![&self.app, &self.helper, &self.cli];
        payloads.extend(self.launcher.as_ref());
        payloads
    }
}

#[derive(Debug)]
pub enum UpdateMsg {
    /// Automatic check completed without user-visible work.
    Finished,
    /// Feed reachable, no newer version. Only sent for manual checks.
    UpToDate { feed_version: String },
    /// No feed configured. Only sent for manual checks.
    NoFeed,
    /// All release payloads were downloaded and SHA-256 validated. No
    /// installed file or process has been changed; applying still needs
    /// explicit user consent.
    Staged(StagedUpdate),
    /// Only sent for manual checks; automatic checks fail silently.
    Error(String),
    /// Automatic check failed; record it in the app error log without opening
    /// a modal during startup/offline use.
    BackgroundError(String),
}
