use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

use super::core::plan;
use super::incremental::SyncEndpoints;
use super::omissions::SyncOmissions;
use super::pair_lock::pair_lock_id;
use super::persistence::{baseline_path, load_baseline, pair_id_for};
use super::run_types::{RunBlock, StateKey};
use super::snapshot::WalkFilter;
use super::snapshot_pair::read_pair;
use super::types::{Action, Baseline, BisyncOptions, BisyncStats, Conflict};

/// Read-only comparison with the same omitted-subtree protection as a real run.
#[derive(Default)]
pub struct Preview {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub a_files: usize,
    pub b_files: usize,
    pub error: Option<String>,
    pub omissions: SyncOmissions,
    pub duplicate_removals: u64,
    /// What a run would stop on before its first change (FS3).
    pub blocked: Option<RunBlock>,
    /// The stored state the preview planned against.
    pub state: Option<StateKey>,
    /// The planned signatures (side A, side B) of every action's rel; a
    /// single action applied from the preview is checked against them (Y148).
    pub planned: Baseline,
}

pub fn preview(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Preview {
    if let Err(error) = crate::vfs::validate_sync_roots(a, root_a, b, root_b) {
        return Preview {
            error: Some(error.to_string()),
            ..Default::default()
        };
    }
    let pair = pair_id_for(a, root_a, b, root_b);
    let state = StateKey::legacy(&pair, &pair_lock_id(a, root_a, b, root_b));
    let base = match load_baseline(&baseline_path(&pair)) {
        Ok(base) => base,
        Err(error) => {
            return Preview {
                error: Some(format!(
                    "Synchronisierungsstand kann nicht gelesen werden: {error}"
                )),
                ..Default::default()
            }
        }
    };
    let snapshot = match read_pair(
        SyncEndpoints::new(a, root_a, b, root_b),
        opts,
        cancel,
        filter,
        &base,
    ) {
        Ok(snapshot) => snapshot,
        Err((path, error)) => {
            return Preview {
                error: Some(format!("{path}: {error}")),
                ..Default::default()
            }
        }
    };
    let (actions, conflicts, _) = snapshot.plan(&base, opts);
    let duplicate_removals = snapshot
        .repairs
        .iter()
        .filter_map(|c| c.duplicates.as_ref())
        .map(|d| d.redundant_count())
        .sum();
    let planned = actions
        .iter()
        .map(action_rel)
        .map(|rel| {
            (
                rel.to_string(),
                (snapshot.a.get(rel).copied(), snapshot.b.get(rel).copied()),
            )
        })
        .collect();
    Preview {
        actions,
        conflicts,
        a_files: snapshot.a.len(),
        b_files: snapshot.b.len(),
        error: None,
        omissions: snapshot.omissions,
        duplicate_removals,
        blocked: None,
        state: Some(state),
        planned,
    }
}

/// Applies one action of `preview` with the planned-state guards of a run,
/// under the pair's lock, and records its result in the preview's state
/// (Y148, "Nur diese Datei jetzt"). Contract stage: not available yet.
#[allow(clippy::too_many_arguments)]
pub fn apply_preview_action(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    preview: &Preview,
    action: &Action,
    opts: BisyncOptions,
    cancel: &AtomicBool,
) -> io::Result<BisyncStats> {
    let _ = (a, root_a, b, root_b, preview, action, opts, cancel);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Einzelne Dateien aus der Vorschau zu übernehmen ist noch nicht verfügbar",
    ))
}

fn action_rel(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel.as_str(),
    }
}
