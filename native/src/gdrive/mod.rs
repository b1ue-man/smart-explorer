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
#[path = "os/shared/binding_store.rs"]
mod binding_store;
#[path = "core/cache.rs"]
mod cache;
#[path = "core/cache_store.rs"]
mod cache_store;
#[path = "core/changes.rs"]
mod changes;
#[path = "core/chunk_stream.rs"]
mod chunk_stream;
#[path = "os/shared/copy_writer.rs"]
mod copy_writer;
#[path = "core/core.rs"]
mod core;
#[path = "core/duplicates.rs"]
mod duplicates;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/file_list.rs"]
mod file_list;
#[path = "core/folder_create_journal.rs"]
mod folder_create_journal;
#[path = "core/http.rs"]
mod http;
#[path = "core/id_pool.rs"]
mod id_pool;
#[path = "core/identity.rs"]
mod identity;
#[path = "core/key_locks.rs"]
mod key_locks;
#[path = "core/listing.rs"]
mod listing;
#[path = "core/listing_query.rs"]
mod listing_query;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/names.rs"]
mod names;
#[path = "core/new_object.rs"]
mod new_object;
#[path = "core/overload.rs"]
mod overload;
#[path = "core/promotion.rs"]
mod promotion;
#[path = "core/promotion_api.rs"]
mod promotion_api;
#[path = "core/promotion_checks.rs"]
mod promotion_checks;
#[path = "core/resolution.rs"]
mod resolution;
#[path = "core/resumable.rs"]
mod resumable;
#[path = "core/resumable_session.rs"]
mod resumable_session;
#[path = "core/sized_writer.rs"]
mod sized_writer;
#[path = "core/stage_time.rs"]
mod stage_time;
#[path = "core/state.rs"]
mod state;
#[path = "core/sync_bindings.rs"]
mod sync_bindings;
#[path = "core/sync_listing.rs"]
mod sync_listing;
#[path = "core/sync_projection.rs"]
mod sync_projection;
#[path = "core/transfer.rs"]
mod transfer;
#[path = "core/transfer_ops.rs"]
mod transfer_ops;
#[path = "core/trash.rs"]
mod trash;

#[cfg(test)]
#[path = "core/mutation_reconcile_tests.rs"]
mod mutation_reconcile_tests;
#[cfg(test)]
#[path = "core/read_retry_tests.rs"]
mod read_retry_tests;
#[cfg(test)]
#[path = "core/remote_provider_task_tests.rs"]
mod remote_provider_task_tests;
#[cfg(test)]
#[path = "core/sync_conflict_task_fixture.rs"]
mod sync_conflict_task_fixture;
#[cfg(test)]
#[path = "core/sync_conflict_task_safety_tests.rs"]
mod sync_conflict_task_safety_tests;
#[cfg(test)]
#[path = "core/sync_conflict_task_tests.rs"]
mod sync_conflict_task_tests;
#[cfg(test)]
#[path = "core/task_drive.rs"]
mod task_drive;
#[cfg(test)]
#[path = "core/task_http.rs"]
mod task_http;
#[cfg(test)]
#[path = "core/transfer_engine_task_ops_tests.rs"]
mod transfer_engine_task_ops_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use state::GDriveBackend;

#[cfg(test)]
#[path = "core/gui_task_fixture.rs"]
mod gui_task_fixture;
#[cfg(test)]
#[path = "core/gui_task_http.rs"]
pub(crate) mod gui_task_http;
#[cfg(test)]
#[path = "core/gui_task_tests.rs"]
mod gui_task_tests;
