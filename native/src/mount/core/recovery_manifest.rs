//! Versioned, offline-readable ownership and import contract. Paths inside
//! journal snapshots remain exactly as persisted, including vault sealing.
use super::types::MountConfig;
use serde::{Deserialize, Serialize};

pub const RECOVERY_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryFile {
    /// Literal components, never an OS path or a backend locator.
    pub components: Vec<String>,
    pub length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryGapKind {
    MissingCache,
    MissingJournal,
    MissingPayload,
    MissingVaultKey,
    MissingBackendIdentity,
    BackendIdentityConflict,
    JournalInventoryUnknown,
    UnreadableJournal,
    TornJournalTail,
    RemoteQuarantineNotBackedUp,
    RemoteStageNotBackedUp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryGap {
    pub kind: RecoveryGapKind,
    /// A persisted path/token, possibly sealed. Never implies saved bytes.
    pub subject: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryJournal {
    pub components: Vec<String>,
    pub entries: Vec<serde_json::Value>,
    pub deletes: Vec<serde_json::Value>,
    pub namespace_intents: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryManifest {
    pub version: u32,
    pub import_version: u32,
    pub recovery_id: String,
    pub created_unix_ms: u64,
    /// Includes backend, connection, root, vault ID and original mount ID.
    pub config: MountConfig,
    /// Stored source/root binding; never requires an online lookup to save.
    /// Google Drive uses RecoveryBackendIdentity. Legacy source-only values
    /// are readable but cannot authorize replay into the current account.
    pub backend_identity: serde_json::Value,
    pub sealed: bool,
    pub files: Vec<RecoveryFile>,
    pub directories: Vec<Vec<String>>,
    pub journals: Vec<RecoveryJournal>,
    pub gaps: Vec<RecoveryGap>,
    /// Every available local byte is included; absent remote bytes are gaps.
    pub local_snapshot_complete: bool,
    /// Original cache remains under this inactive owner's custody.
    pub retained_original_cache: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InactiveMountRecovery {
    pub id: String,
    pub config: MountConfig,
    pub package_path: std::path::PathBuf,
    pub manifest_sha256: String,
    pub retained_cache: std::path::PathBuf,
    pub cache_ids: Vec<super::types::MountId>,
    pub reactivated: bool,
    pub owns_mount_cache: bool,
    pub sealed: bool,
    pub local_snapshot_complete: bool,
    pub gap_count: u64,
    #[serde(default)]
    pub gaps: Vec<RecoveryGap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_error: Option<String>,
}

/// Components are checked on import before any OS operation. Backend paths
/// inside journals are deliberately excluded from this local-path grammar.
pub(crate) fn validate_components(components: &[String]) -> std::io::Result<()> {
    if components.is_empty()
        || components.iter().any(|name| {
            name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0', ':'])
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Recoverypaket enthält einen ungültigen lokalen Dateinamen",
        ));
    }
    Ok(())
}
