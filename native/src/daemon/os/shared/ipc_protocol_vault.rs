//! The requests and replies inside `IpcRequest::Vault { token, req }` and
//! `IpcResponse::Vault` (strand S3, C7.4, VLT-M4). The worker holds the
//! vault keys of the desktop; the GUI, `se` and `se mcp` ask it here, always
//! with the daemon token. `ipc_protocol.rs` holds only the variant pair; a
//! worker of 0.5.182 refuses the variant as unknown. Failures come back as
//! `IpcResponse::Err { msg, code }` with the vault codes of C3.2/C7.5
//! (`vault_locked`, `vault_wrong_password`, `vault_unsupported_format`,
//! `vault_hub`, `vault_memory`, `vault_no_saved_password`,
//! `vault_unsaved_changes`).
//!
//! VLT-M8 adds `Compare`, `CompareText`, `Resolve`, `MergeApply`, `Check`
//! (U26; the comparison of a vault sync job and the check of a site), VLT-M12
//! `SpoolKey` (U14). A comparison that is gone (the service restarted, a newer
//! comparison of the job, the vault locked) answers `compare_stale`.
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// A password or a cleartext item on the loopback IPC (K48): wiped when
/// dropped, never printed. The JSON line that carried it is not wiped.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SecretText(Zeroizing<String>);

impl SecretText {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(..)")
    }
}

impl Serialize for SecretText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// The sealing purposes other processes may use (C3.6); the spool wrap key
/// never leaves the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SealPurposeWire {
    LocalStore,
    LogFields,
}

impl From<SealPurposeWire> for crate::vault::SealPurpose {
    fn from(purpose: SealPurposeWire) -> Self {
        match purpose {
            SealPurposeWire::LocalStore => Self::LocalStore,
            SealPurposeWire::LogFields => Self::LogFields,
        }
    }
}

/// One request to the vault sessions of the worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum VaultIpcRequest {
    /// Every entered site with its state and the newest events (C7.1 T1;
    /// se Desktop polls it, S5 H5).
    Status,
    /// Unlock with a password (T6); `remember` stores or forgets it (T11).
    Unlock {
        site: String,
        password: SecretText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remember: Option<bool>,
    },
    /// Unlock with the remembered password (`--saved-password`).
    UnlockSaved { site: String },
    /// Lock one site or, without `site`, all (T7).
    Lock {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<String>,
        #[serde(default)]
        keep_unsaved: bool,
    },
    /// The cleartext view of an unlocked site: after `Opened` the connection
    /// carries the agent protocol (T8).
    Open { site: String },
    /// Seal cleartext items (protocol fields, local data) for `site`.
    Seal {
        site: String,
        purpose: SealPurposeWire,
        items: Vec<SecretText>,
    },
    /// Unseal items sealed by `Seal`.
    Unseal {
        site: String,
        purpose: SealPurposeWire,
        items: Vec<String>,
    },
    /// VLT-M12 (C3.6, contract V1): the spool key of a vault drive, asked by
    /// its drive host with the host's session token instead of the daemon
    /// token; never the masterkey.
    SpoolKey { mount_id: String },
    /// VLT-M8 (C6): compares the vault job `job` (ID, name or ID prefix).
    /// `groups` are group keys; empty: every group but `structure`. The
    /// answer is the head; the entries stay in the worker and come with
    /// `Items`.
    Compare {
        job: String,
        #[serde(default)]
        groups: Vec<String>,
    },
    /// The entries of the comparison `list` (the job of its head, or
    /// `check:<site>`) of the number `revision` from `offset` on, in pages
    /// that fit the IPC line. Another `revision` is `compare_stale`.
    Items {
        list: String,
        revision: u64,
        offset: u64,
    },
    /// The two versions of a text conflict, for the merge view (T20).
    CompareText { job: String, item: String },
    /// One action on one entry, or with `all` the collective action
    /// (`keep-a`, `keep-b`) on every conflict with two files (T19).
    /// `confirmed`: the user answered the question the action asks.
    Resolve {
        job: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
        action: String,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        confirmed: bool,
    },
    /// The merged text of a text conflict (T20).
    MergeApply {
        job: String,
        item: String,
        content: SecretText,
    },
    /// „Tresor prüfen“ for one site (T21).
    Check { site: String },
}

impl VaultIpcRequest {
    /// Whether the request lifts the restriction of a worker started for the
    /// drive restore (S2 K7): the read-only status of se Desktop does not,
    /// nor does the key request of a drive host the worker started itself.
    pub(crate) fn promotes_daemon(&self) -> bool {
        !matches!(self, Self::Status | Self::SpoolKey { .. })
    }
}

/// The replies of `VaultIpcRequest`, in the same order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum VaultIpcReply {
    Status(VaultStatusView),
    Unlocked {
        site: String,
        epoch: u64,
        /// The site was unlocked already.
        already: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remember_error: Option<String>,
    },
    Locked {
        sites: Vec<String>,
    },
    /// The agent protocol follows on this connection.
    Opened {
        site: String,
        label: String,
        epoch: u64,
        state_identity: String,
        namespace_identity: String,
    },
    Sealed {
        items: Vec<String>,
    },
    Unsealed {
        items: Vec<SecretText>,
    },
    /// The 32-byte spool key as Base64url without padding.
    SpoolKey {
        key: SecretText,
    },
    Compared(CompareHead),
    ItemPage {
        items: Vec<CompareItemWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_offset: Option<u64>,
    },
    /// The versions of a text conflict, cleartext (K48).
    Texts {
        a: SecretText,
        b: SecretText,
    },
    Resolved {
        item: Option<String>,
        /// `ok`.
        result: String,
        resolved: u32,
        #[serde(default)]
        failed: Vec<ResolveFailure>,
    },
    /// The check is kept in the worker as the comparison `check:<site>`.
    Checked {
        site: String,
        revision: u64,
        total: u64,
    },
}

/// What one side holds of an entry (`a` and `b` of the CLI JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompareSideWire {
    pub(crate) exists: bool,
    #[serde(default)]
    pub(crate) size: Option<u64>,
    #[serde(default)]
    pub(crate) mtime_ms: Option<i64>,
    pub(crate) location: String,
}

/// One entry of a comparison or check (keys of `se vault compare --json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompareItemWire {
    pub(crate) item: String,
    pub(crate) group: String,
    pub(crate) kind: String,
    /// The cleartext path; empty for key files and technical entries.
    pub(crate) path: String,
    pub(crate) a: CompareSideWire,
    pub(crate) b: CompareSideWire,
    pub(crate) actions: Vec<String>,
    #[serde(default)]
    pub(crate) finding: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

/// The head of one comparison as it travels (C7.3 `se vault compare
/// --json`): everything but the entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompareHead {
    /// The key of the comparison (`Items.list`).
    pub(crate) job: String,
    pub(crate) site_a: String,
    #[serde(default)]
    pub(crate) label_a: String,
    pub(crate) site_b: String,
    #[serde(default)]
    pub(crate) label_b: String,
    pub(crate) revision: u64,
    pub(crate) counts: BTreeMap<String, usize>,
    /// How many entries `Items` delivers.
    pub(crate) total: u64,
    #[serde(default)]
    pub(crate) unavailable: Option<String>,
}

/// An entry a collective action could not resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResolveFailure {
    pub(crate) item: String,
    pub(crate) error: String,
}

impl From<&crate::vault::SideState> for CompareSideWire {
    fn from(side: &crate::vault::SideState) -> Self {
        Self {
            exists: side.exists,
            size: side.size,
            mtime_ms: side.mtime_ms,
            location: side.location.clone(),
        }
    }
}

impl From<&crate::vault::CompareItem> for CompareItemWire {
    fn from(item: &crate::vault::CompareItem) -> Self {
        Self {
            item: item.id.clone(),
            group: item.group.key().to_string(),
            kind: item.kind.key().to_string(),
            path: item.path.clone(),
            a: CompareSideWire::from(&item.a),
            b: CompareSideWire::from(&item.b),
            actions: item
                .actions
                .iter()
                .map(|action| action.key().to_string())
                .collect(),
            finding: item.finding.clone(),
            note: item.note.clone(),
        }
    }
}

impl From<&crate::vault::CompareView> for CompareHead {
    fn from(view: &crate::vault::CompareView) -> Self {
        Self {
            job: view.job.clone(),
            site_a: view.site_a.clone(),
            label_a: view.label_a.clone(),
            site_b: view.site_b.clone(),
            label_b: view.label_b.clone(),
            revision: view.revision,
            counts: view.counts.clone(),
            total: u64::try_from(view.items.len()).unwrap_or(u64::MAX),
            unavailable: view.unavailable.clone(),
        }
    }
}

#[cfg(test)]
#[path = "ipc_protocol_vault_tests.rs"]
mod tests;
