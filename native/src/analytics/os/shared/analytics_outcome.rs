use super::SizeNode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_SCAN_ISSUES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanStatus {
    Complete,
    Partial,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
}

impl ScanOutcome {
    pub fn complete(tree: SizeNode) -> Self {
        Self {
            tree: Some(tree),
            status: ScanStatus::Complete,
            issues: Vec::new(),
            suppressed_issues: 0,
            permission_denied: 0,
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
        }
    }

    pub fn canceled() -> Self {
        Self {
            tree: None,
            status: ScanStatus::Canceled,
            issues: Vec::new(),
            suppressed_issues: 0,
            permission_denied: 0,
        }
    }
}

#[derive(Default)]
pub(super) struct Diagnostics {
    issues: Mutex<Vec<ScanIssue>>,
    suppressed: AtomicU64,
    root_failed: AtomicBool,
    permission_denied: AtomicU64,
}

impl Diagnostics {
    pub(super) fn record_io(&self, path: impl Into<String>, error: &std::io::Error, is_root: bool) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
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
        }
    }
}
