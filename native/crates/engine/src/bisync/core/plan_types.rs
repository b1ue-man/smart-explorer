//! Inputs and results of planning one pair (V3): the planner sees both sides
//! completely (files, filtered files, folders, omissions) and decides per
//! planning key; everything it does not act on keeps its baseline entry.
use crate::vfs::TargetLimits;

use super::compare::TimeRules;
use super::completion::DirAction;
use super::keys::{KeyPolicy, Spellings};
use super::omissions::SyncOmissions;
use super::snapshot_types::DirSet;
use super::types::{Action, BisyncOptions, Conflict, PairSide, Sig};

/// Everything the planner needs besides both sides and the baseline.
#[derive(Clone, Copy, Debug)]
pub struct PlanContext<'a> {
    pub opts: BisyncOptions,
    pub keys: KeyPolicy,
    pub times: TimeRules,
    /// What side A can store where it receives copies (names, sizes).
    pub limits_a: TargetLimits,
    pub limits_b: TargetLimits,
    /// Equal-size files whose times differ may be read on both sides to
    /// prove them equal (both sides cheap to read: local or hashed by the
    /// server).
    pub can_hash: bool,
    /// Folders present on both sides at the last run; `None` = no folder
    /// history (then folders are only ever added, as a union).
    pub base_dirs: Option<&'a DirSet>,
}

impl PlanContext<'_> {
    /// Exact keys and times, no limits, no hashing, no folder history.
    pub fn new(opts: BisyncOptions) -> Self {
        Self {
            opts,
            keys: KeyPolicy::default(),
            times: TimeRules::EXACT,
            limits_a: TargetLimits::default(),
            limits_b: TargetLimits::default(),
            can_hash: false,
            base_dirs: None,
        }
    }

    pub fn limits(&self, side: PairSide) -> &TargetLimits {
        match side {
            PairSide::A => &self.limits_a,
            PairSide::B => &self.limits_b,
        }
    }
}

/// A baseline entry recorded without an action: (rel, (side A, side B)).
pub type Record = (String, (Option<Sig>, Option<Sig>));

/// The plan of one pair.
#[derive(Clone, Debug, Default)]
pub struct PairPlan {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    /// Equal on both sides, or kept as they are on purpose (one-way extras,
    /// "destination wins"): recorded with the signatures the walk saw.
    pub records: Vec<Record>,
    /// Baseline rels to drop: gone on both sides, or a stale spelling of a
    /// planned key.
    pub forget: Vec<String>,
    /// Equal-size files to read on both sides before planning them again
    /// (`PlanContext::can_hash`).
    pub verify: Vec<String>,
    /// Each side's own spelling of the planned rels.
    pub spellings: Spellings,
    /// Folders to create (parents first) and to remove (deepest first).
    pub dirs: Vec<DirAction>,
    /// Folders present on both sides already (recorded as folder history).
    pub dirs_in_sync: DirSet,
    /// The walks' omissions plus what planning left out (filtered on one
    /// side, colliding names, too large or impossible names on the target).
    pub omissions: SyncOmissions,
    /// Files of each side before the run (deletion guard).
    pub files_a: u64,
    pub files_b: u64,
}
