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
//! VLT-M8 adds `Compare`, `CompareText`, `Resolve`, `MergeApply`, `Check`,
//! VLT-M12 `SpoolKey` (U14, U26).
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
}

impl VaultIpcRequest {
    /// Whether the request lifts the restriction of a worker started for the
    /// drive restore (S2 K7): the read-only status of se Desktop does not.
    pub(crate) fn promotes_daemon(&self) -> bool {
        !matches!(self, Self::Status)
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
}

/// `Vault::Status` (C7.3 `se vault list` keys, S5-V1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VaultStatusView {
    pub(crate) sites: Vec<VaultSiteView>,
    /// Number of the newest event; a poller remembers it.
    #[serde(default)]
    pub(crate) event_seq: u64,
    /// The newest events, oldest first (at most 64).
    #[serde(default)]
    pub(crate) events: Vec<VaultEventView>,
    /// Which lock signals the worker receives on this system.
    #[serde(default)]
    pub(crate) sleep_lock: bool,
    #[serde(default)]
    pub(crate) screen_lock: bool,
    /// What surfaces say next to the screen lock switch (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) screen_lock_note: Option<String>,
}

/// One entered site (never a path inside the vault).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VaultSiteView {
    pub(crate) site: String,
    pub(crate) label: String,
    /// `locked`, `unlocked`, `locking`, `unavailable`.
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) epoch: Option<u64>,
    #[serde(default)]
    pub(crate) consumers: u32,
    #[serde(default)]
    pub(crate) remember_password: bool,
    #[serde(default)]
    pub(crate) auto_lock_idle_min: Option<u32>,
    #[serde(default)]
    pub(crate) lock_on_sleep: bool,
    #[serde(default)]
    pub(crate) lock_on_screen_lock: bool,
    #[serde(default)]
    pub(crate) lock_on_background_min: Option<u32>,
    /// Why an entry is `unavailable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

/// One change of a site (`VaultEvent`, C7.4): site ID, state, reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VaultEventView {
    pub(crate) seq: u64,
    pub(crate) site: String,
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) epoch: u64,
}

#[cfg(test)]
#[path = "ipc_protocol_vault_tests.rs"]
mod tests;
