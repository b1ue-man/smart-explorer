//! Find-and-reclaim scan: local and backend cleanup candidates, large/stale
//! files, empty entries, and duplicate groups. Scans are read-only; UI actions
//! decide what to move to the recycle bin.

mod backend;
mod backend_agent;
mod backend_compare;
mod backend_duplicates;
#[cfg(test)]
mod backend_tests;
mod budget;
mod cleanup;
mod duplicates;
mod finder;
mod finder_compare;
#[cfg(test)]
mod finder_tests;
mod finder_walk;
mod local;
mod retention;
mod stage;
mod types;
mod util;
mod verify;

pub use backend::find_backend_duplicates;
pub use backend::scan_reclaim_backend;
pub use finder::{find_duplicates, DuplicateReport, DuplicateSummary, DuplicateSummaryView};
pub use local::scan_reclaim;
pub use stage::{ReclaimPhase, ReclaimStage};
pub use types::*;
pub use verify::{prepare_reclaim_trash_plan, ReclaimTrashPlan};

pub(crate) use finder::{find_duplicates_in_roots, find_duplicates_in_roots_with_open, candidate_text_budget, FinderLimits, FinderRoot};
