#[path = "os/shared/sync.rs"]
mod imp;
#[path = "os/shared/sync_compare.rs"]
mod sync_compare;
#[path = "os/shared/sync_copy.rs"]
mod sync_copy;
#[path = "os/shared/sync_delete.rs"]
mod sync_delete;
#[path = "os/shared/sync_delete_walk.rs"]
mod sync_delete_walk;
#[path = "os/shared/sync_pass.rs"]
mod sync_pass;
#[path = "os/shared/sync_pass_compat.rs"]
mod sync_pass_compat;
#[path = "os/shared/sync_pass_start.rs"]
mod sync_pass_start;
#[path = "os/shared/sync_run.rs"]
mod sync_run;
#[path = "os/shared/sync_scan.rs"]
mod sync_scan;
#[path = "os/shared/sync_tasks.rs"]
mod sync_tasks;

pub use imp::*;

#[cfg(test)]
#[path = "os/shared/sync_link_tests.rs"]
mod sync_link_tests;
#[cfg(test)]
#[path = "os/shared/sync_parallel_tests.rs"]
mod sync_parallel_tests;
#[cfg(test)]
#[path = "os/shared/sync_robustness_tests.rs"]
mod sync_robustness_tests;
