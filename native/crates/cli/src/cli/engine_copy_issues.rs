//! The issues of a finished engine run (the first hundred come with the
//! result, the log file holds all of them, U49) and the verdict of the run
//! in the terms of `se cp`.
use super::super::entry_kind::EntryKind;
use super::super::tree_observer::TRANSFER_CANCELLED;
use super::super::tree_ops::{Omission, TreeOutcome};
use super::Finished;
use crate::transfer::engine_names::base_name;
use crate::transfer::walk::{omission_kind, OmissionKind};
use crate::transfer::TransferIssue;

/// Failing files listed in the error text before it points at the log.
const SHOWN_FAILURES: usize = 5;

/// Every issue of the run: the engine carries the first hundred, the log file
/// all of them.
pub(super) struct Issues {
    pub(super) items: Vec<TransferIssue>,
    /// Issues counted but neither shown nor readable from the log.
    pub(super) unseen: u64,
}

pub(super) fn all_issues(finished: &Finished) -> Issues {
    let total = finished.progress.errors;
    let shown = finished.issues.len() as u64;
    if total > shown {
        let logged = finished
            .progress
            .log_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_log(&text))
            .unwrap_or_default();
        if logged.len() as u64 >= total {
            return Issues {
                items: logged,
                unseen: 0,
            };
        }
        return Issues {
            items: finished.issues.clone(),
            unseen: total - shown,
        };
    }
    Issues {
        items: finished.issues.clone(),
        unseen: 0,
    }
}

/// The issue log: one JSON object per line. A sealed line (vault) has no
/// readable fields and counts as a failure with a fixed text.
pub(in crate::cli) fn parse_log(text: &str) -> Vec<TransferIssue> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|value| {
            let field = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
            match (field("path"), field("message")) {
                (Some(path), Some(message)) => TransferIssue { path, message },
                _ => TransferIssue {
                    path: String::new(),
                    message: "details of a vault entry are sealed in the log".to_string(),
                },
            }
        })
        .collect()
}

/// The result of a finished run in the terms of `se cp` (U49-4 point 5).
pub(in crate::cli) fn settle(finished: &Finished, dst_path: &str) -> Result<TreeOutcome, String> {
    settle_with(finished, dst_path, true)
}

/// `settle` for a run under `--conflict update`: files left alone and
/// numbered names are what the rule decided, not a destination that changed.
pub(in crate::cli) fn settle_merged(
    finished: &Finished,
    dst_path: &str,
) -> Result<TreeOutcome, String> {
    settle_with(finished, dst_path, false)
}

fn settle_with(finished: &Finished, dst_path: &str, strict: bool) -> Result<TreeOutcome, String> {
    if finished.canceled {
        return Err(TRANSFER_CANCELLED.to_string());
    }
    let skipped = finished.progress.skipped;
    if strict && skipped > 0 {
        return Err(format!(
            "destination changed during copy: {skipped} existing files were left alone"
        ));
    }
    let issues = all_issues(finished);
    let mut omitted = Vec::new();
    let mut failures: Vec<&TransferIssue> = Vec::new();
    for issue in &issues.items {
        let kind = match omission_kind(&issue.message) {
            Some(OmissionKind::Link) => Some(EntryKind::Link),
            Some(OmissionKind::Special) => Some(EntryKind::Special),
            Some(OmissionKind::Other) | None => None,
        };
        match kind {
            Some(kind) if !issue.path.is_empty() => omitted.push(Omission {
                path: issue.path.clone(),
                kind,
                reason: None,
            }),
            _ => failures.push(issue),
        }
    }
    let failed = failures.len() as u64 + issues.unseen;
    if failed > 0 {
        return Err(failure_text(
            failed,
            &failures,
            finished.progress.log_path.as_deref(),
        ));
    }
    if let Some(root) = finished
        .roots
        .iter()
        .find(|root| strict && root.rel != base_name(&root.source))
    {
        // The engine numbers a name that appeared at a remote destination
        // after the check; nothing was replaced, but the name is not the typed one.
        return Err(format!(
            "destination changed during copy: {} appeared at the destination, the data was stored as {} instead",
            base_name(&root.source),
            root.rel
        ));
    }
    Ok(TreeOutcome {
        destination: dst_path.to_string(),
        omitted,
        kept: Vec::new(),
        update: None,
    })
}

fn failure_text(failed: u64, shown: &[&TransferIssue], log: Option<&str>) -> String {
    let mut text = format!("{failed} files failed");
    for issue in shown.iter().take(SHOWN_FAILURES) {
        text.push('\n');
        if issue.path.is_empty() {
            text.push_str(&issue.message);
        } else {
            text.push_str(&format!("{}: {}", issue.path, issue.message));
        }
    }
    if failed > SHOWN_FAILURES as u64 {
        if let Some(log) = log {
            text.push_str(&format!("\nall issues: {log}"));
        }
    }
    text
}
