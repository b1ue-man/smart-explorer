//! Safe two-way (and one-way) sync between two `vfs::Backend`s.
//!
//! Safety is the whole point ("it just works" — the default must be safe):
//!  * A **baseline** from the previous run records each side's state, so we know
//!    which side actually CHANGED — not just which differs. One side changed →
//!    propagate. BOTH sides changed a file → it's a **conflict**, surfaced for
//!    the user; never silently overwritten (strict file-level default).
//!  * Every overwrite/delete is **reversible**: the old bytes are copied into a
//!    versions store first, pruned by a retention window — so any sync action
//!    can be undone.
//!  * `dry_run` reports the plan without touching anything.
//!
//! Backend-agnostic (local↔local, local↔SFTP, …). The line-level git-style
//! merge is a future optional mode; the shipped default is the strict
//! file-level one the spec asks for.
#![allow(dead_code)] // engine; the sync UI wiring lands next.
#![allow(unused_imports)] // re-exports below preserve the crate::bisync API surface.

#[path = "os/shared/apply.rs"]
mod apply;
#[path = "core/plan.rs"]
mod core;
#[path = "os/shared/orchestration.rs"]
mod orchestration;
#[path = "core/paths.rs"]
mod paths;
#[path = "os/shared/persistence.rs"]
mod persistence;
#[path = "os/shared/snapshot.rs"]
mod snapshot;
#[path = "core/types.rs"]
mod types;

pub use apply::{apply, resolve};
pub use core::{plan, update_baseline};
pub use orchestration::{preview, run, Outcome, Preview};
pub use persistence::{
    baseline_path, load_baseline, pair_id, prune_versions, save_baseline, versions_dir,
};
pub use snapshot::{empty_globset, walk_files, HashMode, WalkFilter};
pub use types::{
    Action, Baseline, BisyncOptions, BisyncStats, CompareMode, Conflict, ConflictMode,
    DeletePolicy, Direction, Sig, Throttle, Tree, Versioning, VersioningScheme,
};

#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod tests;
