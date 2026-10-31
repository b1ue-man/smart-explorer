use crate::types::FileEntry;
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[path = "collect_driver.rs"]
mod driver;
#[path = "collect_listing.rs"]
mod listing;

const MAX_COLLECTED_ENTRIES: usize = 1_000_000;
const MAX_COLLECTED_NAME_BYTES: usize = 128 * 1024 * 1024;
const MAX_COLLECTED_DEPTH: u32 = 512;
const MAX_COLLECT_ISSUES: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectIssue {
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct CollectOutcome {
    pub entries: Vec<FileEntry>,
    pub issues: Vec<CollectIssue>,
    pub suppressed_issues: u64,
    pub canceled: bool,
    /// Entries left out as protected omissions (the active app trash on
    /// Android); they never make the outcome incomplete. Always 0 on the
    /// desktop builds.
    pub omitted: u64,
}

impl CollectOutcome {
    pub fn is_complete(&self) -> bool {
        !self.canceled && self.issues.is_empty() && self.suppressed_issues == 0
    }
}

/// The budgets of one collection. Production uses `Limits::PRODUCTION`; the
/// tests use smaller ones on small trees.
#[derive(Clone, Copy, Debug)]
struct Limits {
    entries: usize,
    name_bytes: usize,
    depth: u32,
    issues: usize,
}

impl Limits {
    const PRODUCTION: Limits = Limits {
        entries: MAX_COLLECTED_ENTRIES,
        name_bytes: MAX_COLLECTED_NAME_BYTES,
        depth: MAX_COLLECTED_DEPTH,
        issues: MAX_COLLECT_ISSUES,
    };
}

/// The consented read grant of an explicit copy (`local_access` in production).
trait ReadGrant {
    fn can_request(&self, requested: &str) -> bool;
    fn request(&self, requested: &str) -> Result<bool, String>;
}

struct PlatformGrant;

impl ReadGrant for PlatformGrant {
    fn can_request(&self, requested: &str) -> bool {
        crate::local_access::can_request_access(requested)
    }

    fn request(&self, requested: &str) -> Result<bool, String> {
        crate::local_access::request_access(requested)
    }
}

/// Collect a copy-selection subtree with explicit cancellation, bounded memory,
/// and all-or-nothing completeness reporting. Callers must not apply a move
/// from `entries` unless `is_complete()` is true.
///
/// Folders are listed by listing threads ahead of the walk. The walk takes the
/// listings in its own order, so the entries, the issues and the budgets are
/// those of a walk that lists every folder itself.
pub fn collect_recursive(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
) -> CollectOutcome {
    collect(
        root,
        follow_symlinks,
        start_depth,
        cancel,
        None,
        Limits::PRODUCTION,
        listing::threads(),
    )
}

/// Explicit copy preparation may request one scoped read grant. Background
/// discovery uses `collect_recursive` and never opens a consent dialog.
pub fn collect_recursive_with_access(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
) -> CollectOutcome {
    let grant: &dyn ReadGrant = &PlatformGrant;
    collect(
        root,
        follow_symlinks,
        start_depth,
        cancel,
        Some(grant),
        Limits::PRODUCTION,
        listing::threads(),
    )
}

/// `collect_recursive` with the budgets, the grant and the number of listing
/// threads given (the tests choose them; 0 threads read every folder in the walk).
fn collect(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
    grant: Option<&dyn ReadGrant>,
    limits: Limits,
    threads: usize,
) -> CollectOutcome {
    // A listing thread does not inherit this thread's impersonation token, so a
    // walk that runs under one reads on its own thread only.
    let threads = if crate::local_access::parallel_scan_allowed() {
        threads
    } else {
        0
    };
    listing::run(threads, follow_symlinks, cancel, |listers| {
        driver::walk(
            root,
            follow_symlinks,
            start_depth,
            cancel,
            grant,
            limits,
            listers,
        )
    })
}

fn push_issue(outcome: &mut CollectOutcome, path: &Path, detail: String, limit: usize) {
    if outcome.issues.len() < limit {
        outcome.issues.push(CollectIssue {
            path: path.to_string_lossy().into_owned(),
            detail,
        });
    } else {
        outcome.suppressed_issues = outcome.suppressed_issues.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "collect_order_tests.rs"]
mod order_tests;
