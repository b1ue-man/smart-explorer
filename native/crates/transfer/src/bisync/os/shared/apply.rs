//! Stable apply entrypoints plus streaming reporting for the planner.
pub(super) use super::apply_transfer::back_up;
use super::checkpoint::{ApplyScope, CollectingSink};
use super::completion::DirAction;
use super::incremental::SyncEndpoints;
use super::keys::Spellings;
use super::run_types::StateOwner;
use super::types::{Action, BisyncOptions, BisyncStats, PairSide, Tree};
use super::versions::{RunVersions, VersionsContext};
use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Default, Clone, Debug)]
pub(super) struct ApplyReport {
    pub(super) stats: BisyncStats,
    pub(super) completed: Vec<Action>,
}
#[allow(clippy::too_many_arguments)]
pub fn apply(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> BisyncStats {
    apply_with_results(
        actions,
        a,
        root_a,
        b,
        root_b,
        opts,
        versions_dir,
        errors,
        cancel,
    )
    .stats
}
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_with_results(
    actions: &[Action],
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    legacy(
        actions,
        None,
        SyncEndpoints::new(a, root_a, b, root_b),
        opts,
        versions_dir,
        errors,
        cancel,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_planned_with_results(
    actions: &[Action],
    planned_a: &Tree,
    planned_b: &Tree,
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    legacy(
        actions,
        Some((planned_a, planned_b)),
        SyncEndpoints::new(a, root_a, b, root_b),
        opts,
        versions_dir,
        errors,
        cancel,
    )
}
fn legacy(
    actions: &[Action],
    planned: Option<(&Tree, &Tree)>,
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    versions_dir: &Path,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    let pair = super::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let context = VersionsContext::new(
        &pair,
        StateOwner::AdHoc,
        super::VersionsLocation::AppData,
        opts.versioning,
    );
    let versions = RunVersions::with_app_data(context, versions_dir.to_path_buf());
    let sink = CollectingSink::default();
    let spellings = Spellings::default();
    let scope = ApplyScope {
        sink: &sink,
        versions: &versions,
        spellings: &spellings,
    };
    super::apply_reporting::run(
        actions,
        &[],
        planned,
        endpoints,
        opts,
        &scope,
        errors,
        cancel,
        false,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_planned_reporting(
    actions: &[Action],
    dirs: &[DirAction],
    planned_a: &Tree,
    planned_b: &Tree,
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    super::apply_reporting::run(
        actions,
        dirs,
        Some((planned_a, planned_b)),
        endpoints,
        opts,
        scope,
        errors,
        cancel,
        true,
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_dedupe_reporting(
    candidates: &[crate::vfs::DedupeCandidate],
    side: PairSide,
    planned_a: &Tree,
    planned_b: &Tree,
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    super::apply_dedupe::run(
        candidates, side, planned_a, planned_b, endpoints, opts, scope, errors, cancel,
    )
}
