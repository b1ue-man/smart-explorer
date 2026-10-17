//! Cleartext paths of the vault layer and the storage names they map to:
//! path components, directory cache keys, storage paths below the vault
//! root and the stage-name mapping of C2.6.
//!
//! Stage names (`vfs::split_stage_name`) keep their lead and tail; only the
//! user file part is replaced by its encrypted name, so a mapped stage is
//! still a stage name of the same form (sync rule R9) and never ends in
//! `.c9r`, `.c9s` or `.c9u` (CryptoFS and the vault listing never see it).
//! The mapping is forward only: listings skip stages, stage books compute
//! from the cleartext name.
use std::io;

use crate::cryptomator::{shortened_name, SHORTENED_SUFFIX};

/// Longest name component every supported storage keeps (ext4, f2fs,
/// Android storage: 255 bytes; NTFS, exFAT: 255 UTF-16 units, at least as
/// many as bytes of the ASCII names used here).
pub(crate) const MAX_STORED_NAME: usize = 255;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

/// The components of a cleartext path; `/` and `""` are the vault root.
/// Empty components are skipped, `.` and `..` refused: a vault path is
/// literal and never leaves its folder.
pub(crate) fn components(path: &str) -> io::Result<Vec<&str>> {
    if path.contains('\0') {
        return Err(invalid("Tresorpfad enthält ein NUL-Zeichen"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" => {}
            "." | ".." => return Err(invalid("Tresorpfade enthalten weder . noch ..")),
            name => parts.push(name),
        }
    }
    Ok(parts)
}

/// The cache key of a cleartext folder: its components in NFC (CryptoFS
/// normalizes names before encrypting them, so both spellings are one
/// folder), `/` for the root.
pub(crate) fn dir_key(components: &[&str]) -> String {
    let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfc();
    let mut key = String::new();
    for component in components {
        key.push('/');
        key.push_str(&normalizer.normalize(component));
    }
    if key.is_empty() {
        key.push('/');
    }
    key
}

/// Whether the folder key `key` is `folder` or lies below it.
pub(crate) fn key_within(key: &str, folder: &str) -> bool {
    folder == "/"
        || key == folder
        || key
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The path of `rel` (relative to the vault root) on the backend that
/// stores the vault; `root` is the vault root there.
pub(crate) fn storage_path(root: &str, rel: &str) -> String {
    let root = root.trim_end_matches('/');
    match (root.is_empty(), rel.is_empty()) {
        (true, true) => "/".to_string(),
        (false, true) => root.to_string(),
        (_, false) => format!("{root}/{rel}"),
    }
}

/// `<parent>/<name>` of two vault-root-relative storage paths.
pub(crate) fn child(parent: &str, name: &str) -> String {
    format!("{parent}/{name}")
}

/// The 28 characters `BASE64URL(SHA-1(<encrypted name>))` that also name a
/// shortened entry (`.c9s`), used for stages whose full name would not fit.
fn name_digest(encrypted: &str) -> String {
    let shortened = shortened_name(encrypted);
    shortened
        .strip_suffix(SHORTENED_SUFFIX)
        .unwrap_or(&shortened)
        .to_string()
}

/// How the cleartext stage name `name` is stored, or `None` when `name` is
/// no stage name of `vfs::is_staging_name`.
pub(crate) fn stage_plan(name: &str) -> Option<StagePlan<'_>> {
    crate::vfs::split_stage_name(name).map(|parts| StagePlan {
        lead: parts.lead,
        base: parts.base,
        tail: parts.tail,
    })
}

/// One stage name split around its user file part (C2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StagePlan<'a> {
    lead: &'a str,
    base: Option<&'a str>,
    tail: &'a str,
}

impl<'a> StagePlan<'a> {
    /// The cleartext user file part that is encrypted, if any.
    pub(crate) fn base(&self) -> Option<&'a str> {
        self.base
    }

    /// The stored name: forms without a file part stay literal; otherwise
    /// `encrypt` gives the encrypted name (`<base64>.c9r`) of the file part
    /// and the result is `lead + encrypted + tail` while that fits 255
    /// bytes, else `lead + digest + tail` with the 28-character digest of
    /// the encrypted name.
    pub(crate) fn stored(
        &self,
        encrypt: impl FnOnce(&str) -> io::Result<String>,
    ) -> io::Result<String> {
        let Some(base) = self.base else {
            return Ok(self.tail.to_string());
        };
        let encrypted = encrypt(base)?;
        let full = format!("{}{encrypted}{}", self.lead, self.tail);
        if full.len() <= MAX_STORED_NAME {
            return Ok(full);
        }
        let short = format!("{}{}{}", self.lead, name_digest(encrypted), self.tail);
        if short.len() <= MAX_STORED_NAME {
            Ok(short)
        } else {
            Err(invalid("Stufenname passt nicht in 255 Byte"))
        }
    }
}
