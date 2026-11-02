//! Safety stops before the first change (FS3, B09, Y31/Y62): a mass deletion
//! on one side, the job's absolute limit, a side that lists empty although it
//! had entries. A stop the user confirmed passes once.
use super::run_types::{BlockConfirmation, RunBlock};
use super::types::{Action, Baseline, BisyncOptions, PairSide};

/// Deletions a plan makes on each side. Finishing a verified move and the
/// source deletion of a move job are no deletions (B09).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteCounts {
    pub a: u64,
    pub b: u64,
}

impl DeleteCounts {
    pub fn of(actions: &[Action]) -> Self {
        let mut counts = Self::default();
        for action in actions {
            match action {
                Action::DeleteA(_) => counts.add(PairSide::A, 1),
                Action::DeleteB(_) => counts.add(PairSide::B, 1),
                _ => {}
            }
        }
        counts
    }

    pub fn add(&mut self, side: PairSide, count: u64) {
        match side {
            PairSide::A => self.a = self.a.saturating_add(count),
            PairSide::B => self.b = self.b.saturating_add(count),
        }
    }

    pub fn total(&self) -> u64 {
        self.a.saturating_add(self.b)
    }
}

/// The deletion stop of a plan: the absolute limit (`max_delete`), or at
/// least `max_delete_min` deletions that are at least `max_delete_pct`
/// percent of one side's files.
pub fn deletion_block(
    deletes: DeleteCounts,
    files_a: u64,
    files_b: u64,
    opts: &BisyncOptions,
) -> Option<RunBlock> {
    let total = deletes.total();
    if opts.max_delete > 0 && total > opts.max_delete {
        return Some(RunBlock::DeleteLimit {
            deletes: total,
            limit: opts.max_delete,
        });
    }
    if opts.max_delete_pct == 0 {
        return None;
    }
    let pct = u64::from(opts.max_delete_pct);
    [
        (PairSide::A, deletes.a, files_a),
        (PairSide::B, deletes.b, files_b),
    ]
    .into_iter()
    .find(|(_, deleted, files)| {
        *deleted > 0
            && *deleted >= opts.max_delete_min
            && deleted.saturating_mul(100) >= pct.saturating_mul(*files)
    })
    .map(|(side, deleted, files)| RunBlock::MassDelete {
        side,
        deletes: deleted,
        files,
    })
}

/// Entries the baseline holds for one side (what it had at the last run).
pub fn recorded_entries(base: &Baseline, side: PairSide) -> u64 {
    base.values()
        .filter(|(a, b)| match side {
            PairSide::A => a.is_some(),
            PairSide::B => b.is_some(),
        })
        .count() as u64
}

/// A side that lists empty although the baseline holds entries for it
/// (unmounted drive, emptied folder): independent of the deletion limits.
pub fn empty_side_block(side: PairSide, empty: bool, base: &Baseline) -> Option<RunBlock> {
    if !empty {
        return None;
    }
    let previous = recorded_entries(base, side);
    (previous > 0).then_some(RunBlock::SideEmpty { side, previous })
}

/// `block` unless the user confirmed it for this run.
pub fn unconfirmed(block: Option<RunBlock>, confirmed: &[BlockConfirmation]) -> Option<RunBlock> {
    block.filter(|block| {
        !confirmed
            .iter()
            .any(|confirmation| confirmation.covers(block))
    })
}
