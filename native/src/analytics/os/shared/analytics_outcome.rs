use super::SizeNode;
use crate::analytics::os::display_path;
use crate::analytics::{ProtectedOmission, ProtectedTally};
use crate::apptrash::ProtectedAreas;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_SCAN_ISSUES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScanStatus {
    Complete,
    Partial,
    Failed,
    Canceled,
}

#[cfg(test)]
mod access_tests {
    use super::*;
    fn tree() -> SizeNode {
        SizeNode {
            name: "root".into(),
            size: 7,
            is_dir: true,
            children: Vec::new(),
        }
    }
    #[test]
    fn analytics_access_task_diagnostics_keep_denial_identity_when_report_is_full() {
        let diagnostics = Diagnostics::default();
        for index in 0..70 {
            diagnostics.record_io(
                format!("root/{index}"),
                &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                false,
            );
        }
        diagnostics.record("root/other", "unrelated I/O failure", false);
        let outcome = diagnostics.finish(tree(), false);
        assert_eq!(outcome.status, ScanStatus::Partial);
        assert_eq!(outcome.permission_denied, 70);
        assert_eq!(outcome.issues.len(), 64);
        assert_eq!(outcome.suppressed_issues, 7);
        assert_eq!(outcome.tree.unwrap().size, 7);
    }
    #[test]
    fn analytics_access_task_cancellation_never_becomes_partial_success() {
        let diagnostics = Diagnostics::default();
        diagnostics.record_io(
            "root",
            &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            true,
        );
        let outcome = diagnostics.finish(tree(), true);
        assert_eq!(outcome.status, ScanStatus::Canceled);
        assert!(outcome.tree.is_none());
        assert_eq!(outcome.permission_denied, 0);
        let progress = crate::analytics::Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        let fixture = tempfile::tempdir().unwrap();
        let outcome = crate::analytics::scan(fixture.path(), &progress);
        assert_eq!(outcome.status, ScanStatus::Canceled);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanIssue {
    pub path: String,
    pub detail: String,
}

pub struct ScanOutcome {
    pub tree: Option<SizeNode>,
    pub status: ScanStatus,
    pub issues: Vec<ScanIssue>,
    pub suppressed_issues: u64,
    pub permission_denied: u64,
    /// Informational remarks that do not make the result partial (for
    /// example that a huge directory's files were folded into one node).
    pub notes: Vec<String>,
    /// Files counted into their directory's size without an own tree node.
    pub aggregated_files: u64,
    /// Other apps' private areas (Android) the walk met: what it could not
    /// read there is neither an issue nor a reason for a partial result.
    pub protected: Vec<ProtectedOmission>,
}

impl ScanOutcome {
    pub fn complete(tree: SizeNode) -> Self {
        Self {
            tree: Some(tree),
            status: ScanStatus::Complete,
            issues: Vec::new(),
            suppressed_issues: 0,
            permission_denied: 0,
            notes: Vec::new(),
            aggregated_files: 0,
            protected: Vec::new(),
        }
    }

    pub fn failed(path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            tree: None,
            status: ScanStatus::Failed,
            issues: vec![ScanIssue {
                path: path.into(),
                detail: detail.into(),
            }],
            suppressed_issues: 0,
            permission_denied: 0,
            notes: Vec::new(),
            aggregated_files: 0,
            protected: Vec::new(),
        }
    }

    pub fn canceled() -> Self {
        Self {
            tree: None,
            status: ScanStatus::Canceled,
            issues: Vec::new(),
            suppressed_issues: 0,
            permission_denied: 0,
            notes: Vec::new(),
            aggregated_files: 0,
            protected: Vec::new(),
        }
    }

    /// A root inside a protected area that could not be listed: a complete,
    /// empty result that names the area instead of a failure.
    pub fn protected_root(name: impl Into<Box<str>>, area: impl Into<String>) -> Self {
        let mut outcome = Self::complete(SizeNode {
            name: name.into(),
            size: 0,
            is_dir: true,
            children: Vec::new(),
        });
        outcome.protected.push(ProtectedOmission {
            area: area.into(),
            entries: 1,
        });
        outcome
    }
}

const MAX_SCAN_NOTES: usize = 16;

#[derive(Default)]
pub(super) struct Diagnostics {
    issues: Mutex<Vec<ScanIssue>>,
    suppressed: AtomicU64,
    root_failed: AtomicBool,
    permission_denied: AtomicU64,
    notes: Mutex<Vec<String>>,
    aggregated_files: AtomicU64,
    protected: ProtectedTally,
    /// Other apps' private areas this walk may meet (empty on the desktop).
    areas: ProtectedAreas,
}

impl Diagnostics {
    /// An informational remark; never turns a complete result partial.
    pub(super) fn note(&self, text: impl Into<String>) {
        let mut notes = self.notes.lock().unwrap_or_else(|p| p.into_inner());
        if notes.len() < MAX_SCAN_NOTES {
            notes.push(text.into());
        }
    }

    pub(super) fn with_protected(areas: ProtectedAreas) -> Self {
        Self {
            areas,
            ..Self::default()
        }
    }

    /// Notes a protected area the walk enters; the root may lie inside one.
    pub(super) fn entered(&self, dir: &Path, is_root: bool) {
        if self.areas.is_empty() {
            return;
        }
        let area = if is_root {
            self.areas.area_of(dir)
        } else {
            self.areas.is_area(dir).then_some(dir)
        };
        if let Some(area) = area {
            self.protected.visit(&display_path(area));
        }
    }

    /// A directory that could not be opened or listed: inside a protected
    /// area a counted omission, elsewhere an issue.
    pub(super) fn dir_failed(&self, dir: &Path, error: &io::Error, is_root: bool) {
        match self.areas.area_of(dir) {
            Some(area) => self.protected.omit(&display_path(area)),
            None => self.record_io(display_path(dir), error, is_root),
        }
    }

    /// One entry of `dir` that could not be read. The local directory
    /// adapters name the entry as `<path>: <error>`, which also identifies a
    /// failing `data`/`obb` entry of `<volume>/Android`.
    pub(super) fn entry_failed(&self, dir: &Path, error: &io::Error) {
        if !self.areas.is_empty() {
            let text = error.to_string();
            let area = self.areas.area_of(dir).or_else(|| {
                self.areas.children_of(dir).find(|child| {
                    text.strip_prefix(display_path(child).as_str())
                        .is_some_and(|rest| rest.starts_with(": "))
                })
            });
            if let Some(area) = area {
                self.protected.omit(&display_path(area));
                return;
            }
        }
        self.record_io(display_path(dir), error, false);
    }

    pub(super) fn count_aggregated_files(&self, count: u64) {
        self.aggregated_files.fetch_add(count, Ordering::Relaxed);
    }

    pub(super) fn record_io(&self, path: impl Into<String>, error: &io::Error, is_root: bool) {
        if error.kind() == io::ErrorKind::PermissionDenied {
            self.permission_denied.fetch_add(1, Ordering::Relaxed);
        }
        self.record(path, error.to_string(), is_root);
    }

    pub(super) fn record(&self, path: impl Into<String>, detail: impl Into<String>, is_root: bool) {
        if is_root {
            self.root_failed.store(true, Ordering::Relaxed);
        }
        let issue = ScanIssue {
            path: path.into(),
            detail: detail.into(),
        };
        let mut issues = self.issues.lock().unwrap_or_else(|p| p.into_inner());
        if issues.len() < MAX_SCAN_ISSUES {
            issues.push(issue);
        } else {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn finish(self, tree: SizeNode, canceled: bool) -> ScanOutcome {
        if canceled {
            return ScanOutcome::canceled();
        }
        let root_failed = self.root_failed.load(Ordering::Relaxed);
        let issues = self.issues.into_inner().unwrap_or_else(|p| p.into_inner());
        let suppressed_issues = self.suppressed.load(Ordering::Relaxed);
        let status = if root_failed {
            ScanStatus::Failed
        } else if issues.is_empty() && suppressed_issues == 0 {
            ScanStatus::Complete
        } else {
            ScanStatus::Partial
        };
        ScanOutcome {
            tree: (!root_failed).then_some(tree),
            status,
            issues,
            suppressed_issues,
            permission_denied: self.permission_denied.load(Ordering::Relaxed),
            notes: self.notes.into_inner().unwrap_or_else(|p| p.into_inner()),
            aggregated_files: self.aggregated_files.load(Ordering::Relaxed),
            protected: self.protected.finish(),
        }
    }
}
