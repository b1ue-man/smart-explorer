//! Paths omitted from one snapshot protect the same location on both sides.
//! Every kind of omission protects its counterpart and its baseline entry: an
//! omitted entry is never read as a deletion (V3).
use std::collections::BTreeMap;

use super::keys::KeyPolicy;
use super::types::{Baseline, Tree};

/// Why an entry was left out of a snapshot or of a run (V3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OmissionKind {
    /// Symlink, junction or another redirecting reparse point.
    Link,
    /// Could not be listed, opened or hashed (permission, I/O error).
    Unreadable,
    /// Disappeared between being listed and being read.
    Vanished,
    /// Name this side cannot represent (not UTF-8, invalid character).
    NotRepresentable,
    /// FIFO, socket or device node.
    Special,
    /// Left out by the job's filters (hidden, ignore pattern, size, age).
    Filtered,
    /// The engine's own entries: staging files, `.se-sync-replica`,
    /// `.se-versions`, the app's data, cache and trash folders.
    OwnFile,
    /// Folder owned by the operating system at a volume root (`lost+found`,
    /// `System Volume Information`, `$RECYCLE.BIN`) or another app's private
    /// storage (`Android/data`, `Android/obb`).
    SystemFolder,
    /// Another file system mounted inside the root (`cross_mounts` off).
    Mount,
    /// Larger than the destination file system allows (FAT32: 4 GiB).
    TooLargeForTarget,
    /// The destination cannot hold this name (Windows-forbidden characters,
    /// reserved device names, trailing dot or space).
    NameImpossibleOnTarget,
}

impl OmissionKind {
    pub const ALL: [OmissionKind; 11] = [
        OmissionKind::Link,
        OmissionKind::Unreadable,
        OmissionKind::Vanished,
        OmissionKind::NotRepresentable,
        OmissionKind::Special,
        OmissionKind::Filtered,
        OmissionKind::OwnFile,
        OmissionKind::SystemFolder,
        OmissionKind::Mount,
        OmissionKind::TooLargeForTarget,
        OmissionKind::NameImpossibleOnTarget,
    ];

    /// Stable code for persistence, logs and the Android bridge.
    pub fn as_str(self) -> &'static str {
        match self {
            OmissionKind::Link => "link",
            OmissionKind::Unreadable => "unreadable",
            OmissionKind::Vanished => "vanished",
            OmissionKind::NotRepresentable => "unrepresentable",
            OmissionKind::Special => "special",
            OmissionKind::Filtered => "filtered",
            OmissionKind::OwnFile => "own",
            OmissionKind::SystemFolder => "system",
            OmissionKind::Mount => "mount",
            OmissionKind::TooLargeForTarget => "too_large",
            OmissionKind::NameImpossibleOnTarget => "name_impossible",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == code)
    }

    /// German label for summaries and job results.
    pub fn label(self) -> &'static str {
        match self {
            OmissionKind::Link => "Verknüpfung",
            OmissionKind::Unreadable => "nicht lesbar",
            OmissionKind::Vanished => "während des Laufs verschwunden",
            OmissionKind::NotRepresentable => "Name nicht darstellbar",
            OmissionKind::Special => "Pipe, Socket oder Gerät",
            OmissionKind::Filtered => "gefiltert",
            OmissionKind::OwnFile => "eigene Datei der App",
            OmissionKind::SystemFolder => "Systemordner",
            OmissionKind::Mount => "anderes eingehängtes Dateisystem",
            OmissionKind::TooLargeForTarget => "zu groß für das Ziel-Dateisystem",
            OmissionKind::NameImpossibleOnTarget => "Name auf dem Ziel nicht möglich",
        }
    }

    /// Whether walks report this kind to the user. Filtered entries and the
    /// engine's own files are expected and stay silent; every other kind is
    /// listed (a link the user's filter excludes stays silent as well).
    pub fn reported_by_default(self) -> bool {
        !matches!(self, OmissionKind::Filtered | OmissionKind::OwnFile)
    }
}

impl From<crate::vfs::OmissionReason> for OmissionKind {
    /// The omissions of tolerant listings and hash walks (V1).
    fn from(reason: crate::vfs::OmissionReason) -> Self {
        match reason {
            crate::vfs::OmissionReason::Link => OmissionKind::Link,
            crate::vfs::OmissionReason::Special => OmissionKind::Special,
            crate::vfs::OmissionReason::Unreadable => OmissionKind::Unreadable,
            crate::vfs::OmissionReason::Vanished => OmissionKind::Vanished,
            crate::vfs::OmissionReason::Unrepresentable => OmissionKind::NotRepresentable,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SyncOmissions {
    /// Protected paths by planning key, with the first reason recorded.
    roots: BTreeMap<String, OmissionKind>,
    /// Literal paths, including silent omissions, survive a change of key policy.
    originals: BTreeMap<String, OmissionKind>,
    reported: BTreeMap<String, OmissionKind>,
    keys: KeyPolicy,
}

impl SyncOmissions {
    pub(crate) fn new(fold_case: bool) -> Self {
        Self {
            keys: KeyPolicy { fold_case },
            ..Self::default()
        }
    }

    /// The pair's planning keys (NFC, letter case folded where a side
    /// ignores it), so an omission protects every spelling of its path.
    fn key(&self, path: &str) -> String {
        self.keys.key(path).into_owned()
    }

    /// A link-like or protected entry (the kind every caller recorded before
    /// RV1); see [`SyncOmissions::record_kind`].
    pub(crate) fn record(&mut self, relative: &str, report: bool) {
        self.record_kind(relative, OmissionKind::Link, report);
    }

    /// Protects `relative` (and everything below it) on both sides; `report`
    /// lists it in the run's summary under `kind`.
    pub(crate) fn record_kind(&mut self, relative: &str, kind: OmissionKind, report: bool) {
        let key = self.key(relative);
        self.roots.entry(key).or_insert(kind);
        self.originals.entry(relative.to_string()).or_insert(kind);
        if report {
            self.reported.insert(relative.to_string(), kind);
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        let same_policy = self.keys.fold_case == other.keys.fold_case;
        if same_policy {
            for (path, kind) in other.roots {
                self.roots.entry(path).or_insert(kind);
            }
        }
        for (path, kind) in other.originals {
            if !same_policy {
                let key = self.key(&path);
                self.roots.entry(key).or_insert(kind);
            }
            self.originals.entry(path).or_insert(kind);
        }
        self.reported.extend(other.reported);
    }

    /// Includes ancestors: a file must not replace a directory containing an
    /// omitted child. Component boundaries keep `link-two` independent of `link`.
    pub(crate) fn protects(&self, relative: &str) -> bool {
        let key = self.key(relative);
        if self.contains(relative) {
            return true;
        }
        let prefix = format!("{key}/");
        self.roots
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(path, _)| path.starts_with(&prefix))
    }

    /// Traversal may enter an ancestor to reach independent siblings.
    pub(crate) fn contains(&self, relative: &str) -> bool {
        let key = self.key(relative);
        self.roots.contains_key(&key)
            || key
                .match_indices('/')
                .any(|(index, _)| self.roots.contains_key(&key[..index]))
    }

    pub(crate) fn exclude_tree(&self, tree: &mut Tree) {
        tree.retain(|relative, _| !self.protects(relative));
    }

    pub(crate) fn planning_baseline(&self, original: &Baseline) -> Baseline {
        original
            .iter()
            .filter(|(relative, _)| !self.protects(relative))
            .map(|(relative, signatures)| (relative.clone(), *signatures))
            .collect()
    }

    pub(crate) fn preserve_baseline(&self, original: &Baseline, updated: &mut Baseline) {
        updated.retain(|relative, _| !self.protects(relative));
        updated.extend(
            original
                .iter()
                .filter(|(relative, _)| self.protects(relative))
                .map(|(relative, signatures)| (relative.clone(), *signatures)),
        );
    }

    /// Nothing besides the engine's own entries (replica marker, versions,
    /// staging files) was left out.
    pub fn is_empty(&self) -> bool {
        self.roots
            .values()
            .all(|kind| *kind == OmissionKind::OwnFile)
    }

    pub fn summary(&self) -> Option<String> {
        if self.reported.is_empty() {
            return None;
        }
        let samples: Vec<_> = self
            .reported
            .keys()
            .take(3)
            .map(|path| {
                let mut sample: String = path.chars().take(180).collect();
                if path.chars().count() > 180 {
                    sample.push('…');
                }
                sample
            })
            .collect();
        Some(format!(
            "{} Verknüpfungen oder geschützte Ordner ausgelassen; Gegenstellen unverändert: {}{}",
            self.reported.len(),
            samples.join("; "),
            if self.reported.len() > samples.len() {
                "; …"
            } else {
                ""
            }
        ))
    }

    pub fn reported_paths(&self) -> impl Iterator<Item = &str> {
        self.reported.keys().map(String::as_str)
    }

    pub(crate) fn paths(&self) -> impl Iterator<Item = (&str, OmissionKind, bool)> {
        self.originals
            .iter()
            .map(|(path, kind)| (path.as_str(), *kind, self.reported.contains_key(path)))
    }

    /// Reported paths with the reason each one was left out.
    pub fn reported(&self) -> impl Iterator<Item = (&str, OmissionKind)> {
        self.reported
            .iter()
            .map(|(path, kind)| (path.as_str(), *kind))
    }

    /// Number of reported paths per kind (for job results and the UI).
    pub fn counts(&self) -> BTreeMap<OmissionKind, u64> {
        let mut counts = BTreeMap::new();
        for kind in self.reported.values() {
            *counts.entry(*kind).or_insert(0) += 1;
        }
        counts
    }

    pub fn result_note(&self, status: &str) -> String {
        match self.summary() {
            Some(summary) if status == "ok" => format!("mit Auslassungen; {summary}"),
            Some(summary) => format!("{status}; {summary}"),
            None => status.to_string(),
        }
    }
}
