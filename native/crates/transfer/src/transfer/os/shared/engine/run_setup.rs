//! What a run needs before and when it cannot start: the failure report of a
//! job that ended while it was prepared, the flow of a side, and the packet
//! limits of the first selected source.
use super::super::engine_names::parent_path;
use super::super::flow::{flow_for, local_flow, Flow};
use super::super::job::JobItems;
use super::super::types::{TransferIssue, TransferMsg, TransferProgress};
use super::Side;
use crate::vfs::BatchLimits;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(super) fn report_failure(
    report: &(dyn Fn(TransferMsg) + Sync),
    mut progress: TransferProgress,
    message: String,
    cancel: &AtomicBool,
) {
    progress.done = true;
    progress.discovering = false;
    // A job canceled while it was prepared ends as canceled, as one
    // canceled later does: its preparation stopping is no error of its own.
    if cancel.load(Ordering::Acquire) {
        report(TransferMsg::Done {
            progress,
            errors: Vec::new(),
            canceled: true,
            issues: Vec::new(),
            roots: Vec::new(),
        });
        return;
    }
    progress.errors = 1;
    report(TransferMsg::Done {
        progress,
        errors: vec![message.clone()],
        canceled: false,
        issues: vec![TransferIssue {
            path: String::new(),
            message,
        }],
        roots: Vec::new(),
    });
}

pub(super) fn side_flow(side: Side<'_>, path: &str) -> Arc<Flow> {
    match side {
        Side::Remote(backend) => flow_for(backend, path),
        Side::Local => local_flow(path),
    }
}

/// Packet limits for the files of the selected `first`: below it when it is
/// a folder (an export at a host's root has nothing packable above it),
/// below its parent when it is a file (K-E).
pub(super) fn source_batch_limits(
    backend: &dyn crate::vfs::Backend,
    first: &str,
) -> Option<BatchLimits> {
    backend
        .batch_limits(first)
        .or_else(|| backend.batch_limits(&parent_path(first)))
}

pub(super) fn first_source(items: &JobItems) -> &str {
    match items {
        JobItems::Roots { paths, .. } => paths.first().map(String::as_str),
        JobItems::Pairs(pairs) => pairs.first().map(|pair| pair.source.as_str()),
    }
    .unwrap_or("/")
}
