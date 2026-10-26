//! Generation-bound retry workers; existing views remain installed until merge.
use super::prelude::*;
use crate::scanner::{DeniedRoots, ScanMessage, ScanOpts};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};

pub(super) struct PartialWorker<T> {
    pub generation: u64,
    pub root: String,
    pub rx: Receiver<(String, T)>,
    pub cancel: Arc<AtomicBool>,
}
impl<T> Drop for PartialWorker<T> {
    fn drop(&mut self) { self.cancel.store(true, Ordering::Release); }
}

pub(super) struct ExplorerResult {
    pub entries: Vec<FileEntry>,
    pub progress: ScanProgress,
    pub denied: DeniedRoots,
    pub complete: bool,
    pub measured: bool,
}

pub(super) fn explorer(
    root: String,
    generation: u64,
    roots: Vec<String>,
    max_depth: Option<u32>,
    retention: Option<crate::scanner::RetentionHandle>,
) -> Result<PartialWorker<ExplorerResult>, String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let base = root.clone();
    std::thread::Builder::new().name("denied-subtrees".into()).spawn(move || {
        for path in roots {
            if stop.load(Ordering::Acquire) { break; }
            let Some(depth) = relative_depth(&base, &path) else { continue; };
            if max_depth.is_some_and(|limit| depth >= limit) { continue; }
            let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
            let pin = match crate::scanner::open_retry_root(&base, &path) {
                Ok(pin) => pin,
                Err(_) => continue, // Original root remains retryable and incomplete.
            };
            let handle = crate::scanner::start_scan_confined(PathBuf::from(&path), ScanOpts {
                follow_symlinks: false,
                max_depth: max_depth.map(|limit| limit.saturating_sub(depth)),
                retention: retention.clone(),
            }, scan_tx, pin);
            let mut result = ExplorerResult { entries: Vec::new(),
                progress: super::empty_progress(), denied: Default::default(), complete: false, measured: false };
            loop {
                if stop.load(Ordering::Acquire) {
                    handle.cancel.store(true, Ordering::Release);
                    return;
                }
                match scan_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(ScanMessage::Entries(mut entries)) => {
                        for entry in &mut entries { entry.depth = entry.depth.saturating_add(depth); }
                        result.entries.append(&mut entries);
                    }
                    Ok(ScanMessage::DeniedRoots(denied)) => result.denied = denied,
                    Ok(ScanMessage::Done(progress)) => {
                        result.measured = true;
                        result.complete = progress.errors == 0 &&
                            !handle.truncated.load(Ordering::Acquire);
                        result.progress = progress;
                        break;
                    }
                    Ok(_) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                }
            }
            // Polling a bounded sender keeps cancellation effective while its
            // tab is parked; no unbounded completed-subtree queue accumulates.
            let mut message = (path, result);
            loop {
                match tx.send_timeout(message, std::time::Duration::from_millis(100)) {
                    Ok(()) => break,
                    Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => {
                        if stop.load(Ordering::Acquire) { return; }
                        message = returned;
                    }
                    Err(_) => return,
                }
            }
        }
    }).map_err(|error| error.to_string())?;
    Ok(PartialWorker { generation, root, rx, cancel })
}

pub(super) fn relative_depth(root: &str, path: &str) -> Option<u32> {
    // Retry paths were emitted by the same local scanner and must be descendants.
    if !crate::scanner::retry_contains(root, path) { return None; }
    let segments = |path: &str| path.split(['/', '\\']).filter(|s| !s.is_empty()).count();
    u32::try_from(segments(path).checked_sub(segments(root))?).ok()
}

pub(super) fn merge_entries(entries: &mut Vec<FileEntry>, root: &str,
    replacement: Vec<FileEntry>, complete: bool) {
    let mut positions: HashMap<Arc<str>, usize> = entries.iter().enumerate()
        .map(|(i, entry)| (entry.path.clone(), i)).collect();
    // Incomplete walks only add/update facts, never remove omitted descendants.
    if complete {
        let retained: HashSet<_> = replacement.iter().map(|e| e.path.clone()).collect();
        entries.retain(|e| !crate::scanner::retry_contains(root, &e.path) || retained.contains(&e.path));
        positions = entries.iter().enumerate().map(|(i,e)| (e.path.clone(),i)).collect();
    }
    for entry in replacement {
        if let Some(index) = positions.get(&entry.path) { entries[*index] = entry; }
        else { positions.insert(entry.path.clone(), entries.len()); entries.push(entry); }
    }
}

pub(super) fn analytics(root: String, generation: u64, roots: Vec<String>)
    -> Result<PartialWorker<crate::analytics::ScanOutcome>, String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let progress = crate::analytics::Progress::default();
    let cancel = progress.cancel.clone();
    let base = root.clone();
    std::thread::Builder::new().name("analytics-denied-subtrees".into())
        .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES).spawn(move || {
        for path in roots {
            if progress.check_cancel().is_err() { break; }
            let outcome = crate::scanner::open_retry_root(&base, &path).map(|handle| {
                crate::analytics::scan_confined_with(&PathBuf::from(&path), &progress,
                    &crate::analytics::ScanBudget::with_node_limit(None), &[], handle)
            }).unwrap_or_else(|error| crate::analytics::ScanOutcome::failed(&path, error.to_string()));
            let mut message = (path, outcome);
            loop {
                match tx.send_timeout(message, std::time::Duration::from_millis(100)) {
                    Ok(()) => break,
                    Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => {
                        if progress.check_cancel().is_err() { return; }
                        message = returned;
                    }
                    Err(_) => return,
                }
            }
        }
    }).map_err(|error| error.to_string())?;
    Ok(PartialWorker { root, generation, rx, cancel })
}

/// Preserve known old data for precisely identified omissions in a partial
/// analytics result, then update the retry index's exact measured subtotals.
pub(super) fn prepare_partial_analytics(previous: &crate::analytics::SizeNode,
    base: &str, path: &str, outcome: &mut crate::analytics::ScanOutcome) -> bool {
    if outcome.permission_denied == 0 || outcome.permission_denied !=
        (outcome.issues.len() as u64).saturating_add(outcome.suppressed_issues) { return false; }
    let Some(depth) = relative_depth(base, path) else { return false; };
    let parts: Vec<_> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    let start = parts.len().saturating_sub(depth as usize);
    let Some(previous) = crate::analytics::retry_node(previous, &parts[start..]) else { return false; };
    let Ok(roots) = outcome.denied_roots.roots() else { return false; };
    let Ok(mut measurements) = outcome.denied_roots.measurements() else { return false; };
    let mut omissions = Vec::new();
    for root in roots {
        let Some(depth) = relative_depth(path, &root) else { return false; };
        let parts: Vec<_> = root.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
        let start = parts.len().saturating_sub(depth as usize);
        let segments: Vec<_> = parts[start..].iter().map(|part| (*part).to_owned()).collect();
        let mut ancestor = path.trim_end_matches('/').to_string();
        let sizes: Option<Vec<u64>> = segments.iter().map(|segment| {
            ancestor.push('/'); ancestor.push_str(segment);
            measurements.iter().find(|(measured, _)| same_retry_path(measured, &ancestor))
                .map(|(_, size)| *size)
        }).collect();
        let Some(sizes) = sizes else { return false; };
        omissions.push(crate::analytics::RetryOmission { segments, sizes });
    }
    let Some(replacement) = outcome.tree.as_mut() else { return false; };
    let Some(restored) = crate::analytics::preserve_retry_omissions(previous, replacement, &omissions)
        else { return false; };
    for (segments, before, after) in restored {
        let root = if segments.is_empty() { path.to_owned() }
            else { format!("{}/{}", path.trim_end_matches('/'), segments.join("/")) };
        for (measured, size) in &mut measurements {
            if crate::scanner::retry_contains(measured, &root) {
                *size = size.saturating_sub(before).saturating_add(after);
            }
        }
    }
    for (measured, size) in measurements { outcome.denied_roots.record_size(measured, size); }
    true
}

fn same_retry_path(a: &str, b: &str) -> bool {
    crate::scanner::retry_contains(a, b) && crate::scanner::retry_contains(b, a)
}

#[cfg(test)]
#[path = "batches_partial_scan_task_tests.rs"]
mod batches_task_tests;
