//! The values of the session register (`session.rs`): consumers, states,
//! events, lock requests, lock participants and the status of a site.
use std::fmt;
use std::time::Instant;

use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vault::core::registry_types::SiteRecord;

/// What holds a session besides single operations (C3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsumerKind {
    /// A `Vault::Open` connection: GUI tab, CLI command, transfer.
    Open,
    /// A drive of the site (`MountSource::Vault`) with its mount ID.
    Drive(String),
    /// An open comparison of a vault sync job.
    Compare,
    /// A cleartext copy opened with consent (K05).
    FallbackCopy,
}

/// State of a site as surfaces show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteState {
    Locked,
    Unlocked,
    Locking,
    /// The entry of the vault list cannot be read.
    Unavailable,
}

impl SiteState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Unlocked => "unlocked",
            Self::Locking => "locking",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One change of a site (site ID only, never a name or a path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterEvent {
    pub seq: u64,
    pub site: String,
    pub state: SiteState,
    pub reason: Option<LockReason>,
    pub epoch: u64,
}

/// One lock as the participants see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRequest {
    pub reason: LockReason,
    /// „Sperren, Änderungen verschlüsselt aufbewahren“ (`--keep-unsaved`).
    pub keep_unsaved: bool,
    pub deadline: Instant,
}

impl LockRequest {
    pub fn new(reason: LockReason, keep_unsaved: bool) -> Self {
        Self {
            reason,
            keep_unsaved,
            deadline: Instant::now() + reason.participant_budget(),
        }
    }
}

/// Called before the key of a site is wiped (drives, fallback copies,
/// comparisons; C3.3, S2-V4), by `request.deadline`. A lock someone asked
/// for without `keep_unsaved` first publishes pending work; when that
/// fails, `Err` with what stays unsaved stops the lock („Speichern und
/// sperren“ failed, the participant changes nothing then). Forced locks keep
/// such work sealed; their `Err` changes nothing.
pub trait LockParticipant: Send + Sync {
    fn prepare_lock(&self, site: &str, request: &LockRequest) -> Result<(), String>;
}

/// A lock that did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockError {
    /// Unsaved drive changes could not be published.
    Unsaved(String),
}

impl LockError {
    /// The structured code for IPC `Err.code` and the Bridge.
    pub fn code(&self) -> &'static str {
        "vault_unsaved_changes"
    }
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsaved(what) => write!(
                f,
                "Nicht gespeicherte Laufwerksänderungen ({what}): „Sperren, Änderungen \
                 verschlüsselt aufbewahren“ (--keep-unsaved) oder abbrechen"
            ),
        }
    }
}

impl std::error::Error for LockError {}

/// An entered site in the status of the register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteStatus {
    pub record: SiteRecord,
    pub state: SiteState,
    pub epoch: Option<u64>,
    pub consumers: usize,
}

/// The result of `SessionRegister::install`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Installed {
    pub epoch: u64,
    /// The site was unlocked already; the offered session was not used.
    pub already: bool,
}

/// How a register behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterOptions {
    /// Publish sessions to the process table of `open.rs`, so `vault://`
    /// opens with them.
    pub publish: bool,
    pub platform: LockPlatform,
    /// Start the timer thread and the platform signals at the first unlock.
    pub background: bool,
}
