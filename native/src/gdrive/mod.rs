//! Google Drive backend (#19, slice 2) - `impl vfs::Backend` over the Drive v3
//! REST API, so Drive plugs into the same browse/scan/sync machinery as SFTP &
//! co. Auth (PKCE OAuth, token refresh) lives in `cloud.rs`; this module only
//! makes authenticated REST calls.
//!
//! Drive is ID-addressed, not path-addressed, so we keep a `path -> fileId`
//! cache and resolve lazily from the My-Drive root (`"root"`). Forward-slash
//! paths are the app's convention; `"/"` is the Drive root.
//!
//! NOTE: this code follows the documented Drive v3 API but cannot be exercised
//! in the headless build env (no OAuth client). It compiles for host +
//! windows-gnu and is gated behind an explicit, user-configured connection.

#[path = "core/api.rs"]
mod api;
#[path = "core/auth.rs"]
mod auth;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/cache.rs"]
mod cache;
#[path = "core/changes.rs"]
mod changes;
#[path = "core/core.rs"]
mod core;
#[path = "core/dedupe.rs"]
mod dedupe;
#[path = "core/folder_create_journal.rs"]
mod folder_create_journal;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/promotion.rs"]
mod promotion;
#[path = "core/promotion_api.rs"]
mod promotion_api;
#[path = "core/resumable.rs"]
mod resumable;
#[path = "core/state.rs"]
mod state;
#[path = "core/transfer.rs"]
mod transfer;
#[path = "core/trash.rs"]
mod trash;

#[cfg(test)]
#[path = "core/mutation_reconcile_tests.rs"]
mod mutation_reconcile_tests;
#[cfg(test)]
#[path = "core/read_retry_tests.rs"]
mod read_retry_tests;

pub use state::GDriveBackend;
