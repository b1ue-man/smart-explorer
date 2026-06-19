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
#[path = "core/core.rs"]
mod core;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/state.rs"]
mod state;
#[path = "core/transfer.rs"]
mod transfer;

pub use state::GDriveBackend;
