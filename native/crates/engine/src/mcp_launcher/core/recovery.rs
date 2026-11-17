//! Which way an update transaction goes whose owner ended before Commit or
//! Rollback. The installed regular files decide: all of them with their
//! previous content roll back, all of them with the target content complete
//! (after the caller's start proof), every other combination is refused and
//! named per file. Pure: the caller hashes the files and holds the locks.
use super::lifecycle::{InstallationTarget, UpdateState};

/// One regular installation file as found, against its two recorded hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundTarget {
    pub file: String,
    /// SHA-256 of the installed file; `None`: the file does not exist.
    pub sha256: Option<String>,
    /// The file holds its previous content (absent where it was absent).
    pub previous: bool,
    /// The file holds the target content.
    pub target: bool,
}

/// The direction the installed files allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Every file still holds its previous content: roll back.
    RollBack,
    /// Every file holds the target content: complete after the start proof.
    Complete,
    /// The files hold neither one version nor the other (each one listed).
    Refuse(Vec<FoundTarget>),
}

/// `target` as found with `sha256` (`None`: absent).
pub fn found(target: &InstallationTarget, sha256: Option<String>) -> FoundTarget {
    FoundTarget {
        previous: sha256.as_deref() == target.previous_sha256.as_deref(),
        target: sha256.as_deref() == Some(target.target_sha256.as_str()),
        file: target.file.clone(),
        sha256,
    }
}

/// The direction for `state` from the files found. A recorded file missing
/// from `found` counts as unknown content; a record without files is refused.
pub fn resolve(state: &UpdateState, found: &[FoundTarget]) -> Resolution {
    let entries: Vec<FoundTarget> = state
        .installation_targets
        .iter()
        .map(|target| {
            found
                .iter()
                .find(|entry| entry.file == target.file)
                .cloned()
                .unwrap_or_else(|| FoundTarget {
                    file: target.file.clone(),
                    sha256: None,
                    previous: false,
                    target: false,
                })
        })
        .collect();
    if entries.is_empty() {
        Resolution::Refuse(entries)
    } else if entries.iter().all(|entry| entry.previous) {
        Resolution::RollBack
    } else if entries.iter().all(|entry| entry.target) {
        Resolution::Complete
    } else {
        Resolution::Refuse(entries)
    }
}

/// The refusal of a mixed state: the code `update_recovery_refused`, then
/// each file with the version it holds.
pub fn refusal(state: &UpdateState, entries: &[FoundTarget]) -> String {
    let (previous, target) = (
        &state.previous.product_version,
        &state.target.product_version,
    );
    format!(
        "update_recovery_refused: die Programmdateien gehören weder vollständig zu v{previous} \
         noch vollständig zu v{target}: {}",
        file_states(state, entries)
    )
}

/// Each file with the version it holds, joined by "; ".
pub fn file_states(state: &UpdateState, entries: &[FoundTarget]) -> String {
    let (previous, target) = (
        &state.previous.product_version,
        &state.target.product_version,
    );
    if entries.is_empty() {
        return "keine Datei aufgezeichnet".to_string();
    }
    entries
        .iter()
        .map(|entry| describe(entry, previous, target))
        .collect::<Vec<_>>()
        .join("; ")
}

fn describe(entry: &FoundTarget, previous: &str, target: &str) -> String {
    let file = &entry.file;
    match (&entry.sha256, entry.previous, entry.target) {
        (_, true, true) => format!("{file} gleich in v{previous} und v{target}"),
        (None, true, false) => format!("{file} fehlt wie in v{previous}"),
        (_, true, false) => format!("{file} ist v{previous}"),
        (_, false, true) => format!("{file} ist v{target}"),
        (None, false, false) => format!("{file} fehlt"),
        (Some(sha256), false, false) => format!(
            "{file} hat unbekannten Inhalt (SHA-256 {})",
            sha256.get(..12).unwrap_or(sha256)
        ),
    }
}
