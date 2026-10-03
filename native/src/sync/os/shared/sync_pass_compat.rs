//! Compatibility copy-pass entrypoint with its own pair lock and run versions.
use super::imp::{record_error, SyncMsg};
use super::sync_pass::{copy_pass_scoped, Report};
use super::sync_scan::Target;
use crate::vfs::Backend;
use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_pass(
    src: &dyn Backend,
    src_root: &str,
    dst: &dyn Backend,
    dst_root: &str,
    dry_run: bool,
    root: Target,
    cancel: &AtomicBool,
    mut report: Report,
    tx: &Sender<SyncMsg>,
    start: Instant,
) -> Report {
    let run = match super::sync_run::MirrorRun::begin(src, src_root, dst, dst_root) {
        Ok(run) => run,
        Err(error) => {
            record_error(
                &mut report.stats,
                &mut report.errors,
                dst_root,
                error.to_string(),
            );
            return report;
        }
    };
    report = copy_pass_scoped(
        src,
        src_root,
        dst,
        dst_root,
        dry_run,
        root,
        cancel,
        report,
        tx,
        start,
        &run.versions,
    );
    if let Err(error) = run.finish(dst, dst_root, cancel) {
        record_error(
            &mut report.stats,
            &mut report.errors,
            dst_root,
            error.to_string(),
        );
    }
    report
}
