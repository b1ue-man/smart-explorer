//! Historical authority for cached writes. A source selection is not proof
//! of the provider account or of the directory object originally mounted.
use super::types::MountSource;
use serde::{Deserialize, Serialize};
use std::io;

pub(crate) const IDENTITY_FILE: &str = "recovery-identity.json";
pub(crate) const IDENTITY_MISSING: &str = "mount_recovery_identity_missing";
pub(crate) const IDENTITY_CONFLICT: &str = "mount_recovery_identity_conflict";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryBackendIdentity {
    pub version: u32,
    pub source: MountSource,
    pub provider: String,
    /// Non-secret hash of the provider-confirmed principal, never OAuth data.
    pub principal: String,
    pub namespace: String,
    pub state_identity: String,
    /// Immutable provider ID, never a display path or the `root` alias.
    pub root_object: String,
}

impl RecoveryBackendIdentity {
    pub(crate) fn validate_for(&self, source: &MountSource) -> io::Result<()> {
        if self.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Die Version der historischen Laufwerkidentität wird nicht unterstützt",
            ));
        }
        if !matches!(source, MountSource::GoogleDrive { .. })
            || &self.source != source
            || self.provider != "gdrive"
            || self.principal.len() != 64
            || !self.principal.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.namespace != format!("gdrive:path-v2:{}", self.principal)
            || !self
                .state_identity
                .starts_with(&format!("{}:", self.namespace))
            || self.root_object.is_empty()
            || self.root_object == "root"
        {
            return Err(conflict());
        }
        Ok(())
    }

    /// Check the principal before any root lookup in the fresh connection.
    pub(crate) fn matches_session(&self, state: &str, namespace: &str) -> io::Result<()> {
        if self.state_identity != state || self.namespace != namespace {
            return Err(conflict());
        }
        Ok(())
    }
}

/// Legacy packages remain readable and savable. Their source-only value
/// grants no authority to replay writes into today's signed-in account.
pub(crate) fn manifest_identity(
    value: &serde_json::Value,
    source: &MountSource,
) -> io::Result<Option<RecoveryBackendIdentity>> {
    if *value == serde_json::to_value(source).map_err(io::Error::other)? {
        return Ok(None);
    }
    let identity: RecoveryBackendIdentity =
        serde_json::from_value(value.clone()).map_err(|_| conflict())?;
    identity.validate_for(source)?;
    Ok(Some(identity))
}

pub(crate) fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "{IDENTITY_MISSING}: Historischer Nachweis für Google-Drive-Konto und Rootobjekt fehlt. Paket und Cache bleiben erhalten. Einen belegten ursprünglichen Cache mit recovery-identity.json wiederherstellen; eine erneute Anmeldung allein ordnet diese Schreibreste nicht zu"))
}

pub(crate) fn conflict() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "{IDENTITY_CONFLICT}: Google-Drive-Konto, Namespace oder Rootobjekt widerspricht der ursprünglichen Cachebindung. Mit dem ursprünglichen Konto und Root erneut versuchen; Paket und Cache bleiben erhalten"))
}
