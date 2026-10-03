//! Construct one mirror pass and hand its queues to the worker coordinator.
use super::imp::{record_error, SyncMsg, WalkBudget};
use super::sync_pass::{Crew, Pass, Report, State};
use super::sync_scan::{DirTask, Target};
use crate::bisync::KeyPolicy;
use crate::bisync::sync_flows::{PairFlows, PairSide};
use crate::bisync::sync_overload::Progress;
use crate::bisync::versions::RunVersions;
use crate::transfer::engine::folders::FolderRegister;
use crate::transfer::Side;
use crate::vfs::{Backend, Scheme};
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_pass_scoped(
    src: &dyn Backend,
    src_root: &str,
    dst: &dyn Backend,
    dst_root: &str,
    dry_run: bool,
    root: Target,
    cancel: &AtomicBool,
    report: Report,
    tx: &Sender<SyncMsg>,
    start: Instant,
    versions: &RunVersions,
) -> Report {
    let flows = PairFlows::new(src, src_root, dst, dst_root);
    let folders = FolderRegister::new(Side::Remote(dst), dst_root, flows.flow(PairSide::B).clone());
    let copy_cap = flows.shared_connection_cap(src, dst).unwrap_or(usize::MAX);
    let confirm_listing = !matches!(dst.scheme(), Scheme::Ftp | Scheme::Webdav | Scheme::GDrive);
    let pass = Pass {
        src,
        src_root,
        dst,
        dst_root,
        dry_run,
        keys: KeyPolicy::for_pair(src.case_sensitive_paths(src_root), dst.case_sensitive_paths(dst_root)),
        versions,
        stopped: AtomicBool::new(false),
        cancel,
        flows,
        folders,
        copy_cap,
        progress: Progress::default(),
        guard_parent: dst.is_local(),
        confirm_listing,
        state: Mutex::new(State {
            dirs: VecDeque::new(),
            files: VecDeque::new(),
            scanners: Crew::default(),
            copiers: Crew::default(),
            scan_stopped: false,
            finished: false,
            budget: WalkBudget::default(),
            report,
            current: String::new(),
        }),
        changed: Condvar::new(),
        streaming: AtomicU64::new(0),
    };
    {
        let mut guard = pass.lock();
        let state = &mut *guard;
        match state.budget.record("", 0) {
            Ok(()) => state.dirs.push_back(DirTask {
                path: src_root.to_string(),
                rel: String::new(),
                target_rel: String::new(),
                depth: 0,
                target: root,
            }),
            Err(error) => record_error(
                &mut state.report.stats,
                &mut state.report.errors,
                src_root,
                error,
            ),
        }
    }
    std::thread::scope(|scope| pass.coordinate(scope, tx, start));
    let state = pass
        .state
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.report
}
