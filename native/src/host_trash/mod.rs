//! Host-owned reversible Windows trash, with a visible local restore facade.
#[cfg(windows)]
#[path = "os/shared/catalog.rs"]
mod catalog;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "os/unsupported.rs"]
mod platform;
#[path = "core/record.rs"]
mod record;
#[cfg(windows)]
#[path = "os/shared/restore.rs"]
mod restore;
#[cfg(all(windows, test))]
#[path = "os/shared/review_task_tests.rs"]
mod review_task_tests;
#[cfg(windows)]
#[path = "os/shared/store.rs"]
mod store;

pub(crate) use platform::{available, list, recycle_selected, restore};
pub(crate) use record::{CatalogEntry, CatalogPage, EntryState, RestoreOutcome};
