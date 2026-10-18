//! Path fields of the drive journal and the stage ledger (contract V1,
//! S2-V3). The engine keeps every path in plain text in memory and passes
//! each persisted path field through a `PathSealer` on its way to disk and
//! back: `remote_path`, `original_path`, `quarantine_path`, the paths of
//! namespace intents and conflicts, and the stage paths of `stages.json`.
//! The plain sealer is the identity, so a drive without a vault writes the
//! bytes of every earlier release. A vault drive uses the sealed text form
//! of S3 (`N/vault/os/shared/journal_sealer.rs`).
//!
//! The free text of a conflict (`detail`) can name paths too. It passes
//! `seal_detail`/`unseal_detail`, which keep it as it is unless a sealer
//! seals texts as well: a vault drive does (VLT-M12, decision on U02 open
//! point 5), every other sealer keeps the text and its bytes.
use std::io;

pub trait PathSealer: Send + Sync {
    /// The persisted form of one plain path.
    fn seal(&self, path: &str) -> io::Result<String>;
    /// The plain path of one persisted form; fails for a form this sealer did
    /// not write (wrong key, damaged text).
    fn unseal(&self, sealed: &str) -> io::Result<String>;
    /// The persisted form of a conflict text; identity by default.
    fn seal_detail(&self, text: &str) -> io::Result<String> {
        Ok(text.to_string())
    }
    /// The conflict text of one persisted form; identity by default.
    fn unseal_detail(&self, sealed: &str) -> io::Result<String> {
        Ok(sealed.to_string())
    }
}

/// Identity: persisted paths are the plain paths (every non-vault drive).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlainPathSealer;

impl PathSealer for PlainPathSealer {
    fn seal(&self, path: &str) -> io::Result<String> {
        Ok(path.to_string())
    }

    fn unseal(&self, sealed: &str) -> io::Result<String> {
        Ok(sealed.to_string())
    }
}
