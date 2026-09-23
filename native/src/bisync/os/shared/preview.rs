use crate::vfs::Backend;
use std::sync::atomic::AtomicBool;

use super::core::plan;
use super::incremental::SyncEndpoints;
use super::omissions::SyncOmissions;
use super::persistence::{baseline_path, load_baseline, pair_id_for};
use super::snapshot::WalkFilter;
use super::snapshot_pair::read_pair;
use super::types::{Action, BisyncOptions, Conflict};

/// Read-only comparison with the same omitted-subtree protection as a real run.
#[derive(Default)]
pub struct Preview {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub a_files: usize,
    pub b_files: usize,
    pub error: Option<String>,
    pub omissions: SyncOmissions,
}

pub fn preview(
    a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str,
    opts: BisyncOptions, cancel: &AtomicBool, filter: &WalkFilter,
) -> Preview {
    if let Err(error) = crate::vfs::validate_sync_roots(a, root_a, b, root_b) {
        return Preview { error: Some(error.to_string()), ..Default::default() };
    }
    let base = match load_baseline(&baseline_path(&pair_id_for(a, root_a, b, root_b))) {
        Ok(base) => base,
        Err(error) => return Preview {
            error: Some(format!("Synchronisierungsstand kann nicht gelesen werden: {error}")),
            ..Default::default()
        },
    };
    let snapshot = match read_pair(SyncEndpoints::new(a, root_a, b, root_b), opts, cancel, filter, &base) {
        Ok(snapshot) => snapshot,
        Err((path, error)) => return Preview {
            error: Some(format!("{path}: {error}")), ..Default::default()
        },
    };
    let base = snapshot.omissions.planning_baseline(&base);
    let (actions, conflicts, _) = plan(&snapshot.a, &snapshot.b, &base, opts);
    Preview {
        actions, conflicts, a_files: snapshot.a.len(), b_files: snapshot.b.len(),
        error: None, omissions: snapshot.omissions,
    }
}
