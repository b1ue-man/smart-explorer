#[path = "os/shared/sync.rs"]
mod imp;
#[path = "os/shared/sync_copy.rs"]
mod sync_copy;
#[path = "os/shared/sync_delete.rs"]
mod sync_delete;
#[path = "os/shared/sync_pass.rs"]
mod sync_pass;
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
