//! Find-and-reclaim scan: local and backend cleanup candidates, large/stale
//! files, empty entries, and duplicate groups. Scans are read-only; UI actions
//! decide what to move to the recycle bin.

mod backend;
#[cfg(test)]
mod backend_tests;
mod cleanup;
mod duplicates;
mod local;
mod types;
mod util;
mod verify;

pub use backend::scan_reclaim_backend;
pub use local::scan_reclaim;
pub use types::*;
pub use verify::{prepare_reclaim_trash_plan, ReclaimTrashPlan};
