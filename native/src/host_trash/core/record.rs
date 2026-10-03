//! Durable intent data; OS adapters alone decode the exact path units.
use serde::{Deserialize, Serialize};
use std::io;

pub(super) const VERSION: u32 = 1;
pub(super) const MAX_PATH_UNITS: usize = 32_767;
pub(super) const MAX_RECORD_BYTES: u64 = (2 * MAX_PATH_UNITS * 6 + 8192) as u64;
pub(super) const PAGE_SIZE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdentity {
    pub(super) volume: u64,
    pub(super) file: [u8; 16],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub(super) version: u32,
    pub(super) id: String,
    pub(super) created_ms: i64,
    pub(super) root: Vec<u16>,
    pub(super) relative: Vec<Vec<u16>>,
    pub(super) root_identity: FileIdentity,
    pub(super) file_identity: FileIdentity,
    pub(super) size: u64,
    pub(super) sha256: String,
    /// Both names are durable before capture, including the restore hop.
    pub(super) held: String,
    pub(super) restore_held: String,
}

pub(super) fn lower_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn held_name(name: &str) -> bool {
    name.strip_prefix(".held.se-recycle-").is_some_and(|nonce| lower_hex(nonce, 16))
}

impl Record {
    pub(super) fn validate(&self) -> io::Result<()> {
        let total = self.relative.iter().try_fold(self.root.len(), |sum, name|
            sum.checked_add(name.len()).and_then(|sum| sum.checked_add(1)));
        let valid_component = |name: &Vec<u16>| !name.is_empty()
            && name.as_slice() != &[46] && name.as_slice() != &[46, 46]
            && !name.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92));
        if self.version != VERSION || !lower_hex(&self.id, 32)
            || self.root.is_empty() || self.root.contains(&0)
            || self.relative.is_empty() || !self.relative.iter().all(valid_component)
            || total.is_none_or(|units| units > MAX_PATH_UNITS)
            || !lower_hex(&self.sha256, 64)
            || !held_name(&self.held) || !held_name(&self.restore_held)
            || self.held == self.restore_held
            || self.root_identity.volume == 0 || self.root_identity.file == [0; 16]
            || self.file_identity.volume == 0 || self.file_identity.file == [0; 16]
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Ungültiger Papierkorb-Intent"));
        }
        Ok(())
    }

    pub(super) fn slots(&self) -> [&str; 2] { [&self.held, &self.restore_held] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryState {
    Held,
    RestorePending,
    OriginalPresent,
    Missing,
    Problem,
}
impl EntryState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Held => "Im Smart-Explorer-Papierkorb",
            Self::RestorePending => "Wiederherstellung unterbrochen",
            Self::OriginalPresent => "Am Originalort",
            Self::Missing => "Inhalt nicht gefunden",
            Self::Problem => "Prüfung erforderlich",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogEntry {
    pub(crate) id: String,
    pub(crate) original: String,
    pub(crate) size: Option<u64>,
    pub(crate) created_ms: Option<i64>,
    pub(crate) state: EntryState,
    pub(crate) detail: Option<String>,
}
impl CatalogEntry {
    pub(crate) fn can_restore(&self) -> bool {
        matches!(self.state, EntryState::Held | EntryState::RestorePending)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogPage {
    pub(crate) entries: Vec<CatalogEntry>,
    pub(crate) next: Option<String>,
    pub(crate) issues: Vec<String>,
    pub(crate) suppressed_issues: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreOutcome { Restored, AlreadyAtOriginal }
