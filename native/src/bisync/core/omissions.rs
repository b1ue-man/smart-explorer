//! Paths omitted from one snapshot protect the same location on both sides.
use std::collections::BTreeSet;

use super::types::{Baseline, Tree};

#[derive(Clone, Debug, Default)]
pub struct SyncOmissions {
    roots: BTreeSet<String>,
    reported: BTreeSet<String>,
    fold_case: bool,
}

impl SyncOmissions {
    pub(crate) fn new(fold_case: bool) -> Self {
        Self { fold_case, ..Self::default() }
    }

    fn key(&self, path: &str) -> String {
        if self.fold_case { path.to_lowercase() } else { path.to_string() }
    }

    pub(crate) fn record(&mut self, relative: &str, report: bool) {
        self.roots.insert(self.key(relative));
        if report {
            self.reported.insert(relative.to_string());
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for path in other.roots {
            self.roots.insert(self.key(&path));
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
        self.roots.range(prefix.clone()..).next().is_some_and(|path| path.starts_with(&prefix))
    }

    /// Traversal may enter an ancestor to reach independent siblings.
    pub(crate) fn contains(&self, relative: &str) -> bool {
        let key = self.key(relative);
        self.roots.contains(&key)
            || key.match_indices('/').any(|(index, _)| self.roots.contains(&key[..index]))
    }

    pub(crate) fn exclude_tree(&self, tree: &mut Tree) {
        tree.retain(|relative, _| !self.protects(relative));
    }

    pub(crate) fn planning_baseline(&self, original: &Baseline) -> Baseline {
        original.iter().filter(|(relative, _)| !self.protects(relative))
            .map(|(relative, signatures)| (relative.clone(), *signatures)).collect()
    }

    pub(crate) fn preserve_baseline(&self, original: &Baseline, updated: &mut Baseline) {
        updated.retain(|relative, _| !self.protects(relative));
        updated.extend(original.iter().filter(|(relative, _)| self.protects(relative))
            .map(|(relative, signatures)| (relative.clone(), *signatures)));
    }

    pub fn is_empty(&self) -> bool { self.roots.is_empty() }

    pub fn summary(&self) -> Option<String> {
        if self.reported.is_empty() { return None; }
        let samples: Vec<_> = self.reported.iter().take(3)
            .map(|path| {
                let mut sample: String = path.chars().take(180).collect();
                if path.chars().count() > 180 { sample.push('…'); }
                sample
            }).collect();
        Some(format!("{} Verknüpfungen ausgelassen; Gegenstellen unverändert: {}{}",
            self.reported.len(), samples.join("; "),
            if self.reported.len() > samples.len() { "; …" } else { "" }))
    }

    pub fn reported_paths(&self) -> impl Iterator<Item = &str> {
        self.reported.iter().map(String::as_str)
    }

    pub fn result_note(&self, status: &str) -> String {
        match self.summary() {
            Some(summary) if status == "ok" => format!("mit Auslassungen; {summary}"),
            Some(summary) => format!("{status}; {summary}"),
            None => status.to_string(),
        }
    }
}
