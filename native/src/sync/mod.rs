#[path = "os/shared/sync.rs"]
mod imp;
#[path = "os/shared/sync_copy.rs"]
mod sync_copy;
#[path = "os/shared/sync_delete.rs"]
mod sync_delete;

pub use imp::*;
