//! SMB 2/3 backend (`smb2`, pure Rust) implementing `vfs::Backend`.
//!
//! Endpoint `smb://user@host:port/<share>/<path>`: the first path segment is
//! the share, the rest a path inside it; `DOMAIN\user` in the user field is
//! the NTLM domain. Password logins only; DFS is never followed.
//!
//! Async↔sync bridge like SFTP: a private multi-threaded tokio runtime per
//! backend (its workers drive smb2's receiver and keepalive tasks) and one
//! `block_on` per blocking `Backend` call. A dead connection is replaced by a
//! new session; each share gets its tree connect lazily on first use.
//!
//! Listing, stat, delete and rename use the public message API instead of
//! smb2's convenience methods: those drop the file attributes, so a reparse
//! point (symlink, junction) would look like a plain folder, and smb2's
//! rename can only refuse an existing target. Here a reparse point is a link
//! that recursive delete and sync never enter, a link is deleted and renamed
//! itself (never its target), and replacing is one server-side rename with
//! `ReplaceIfExists`. Copies inside one share run on the server (COPYCHUNK,
//! server_copy.rs). A folder is deleted with one listing per folder and
//! concurrent deletes under the credit window (tree_remove.rs).

#[path = "core/backend.rs"]
mod backend;
#[path = "core/batch.rs"]
mod batch;
#[path = "core/batch_wire.rs"]
mod batch_wire;
#[path = "core/compound_put.rs"]
mod compound_put;
#[path = "core/compound_read.rs"]
mod compound_read;
#[path = "core/credit_retry.rs"]
mod credit_retry;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/listing.rs"]
mod listing;
#[path = "core/reader.rs"]
mod reader;
#[path = "core/replace.rs"]
mod replace;
#[path = "core/segment_wire.rs"]
mod segment_wire;
#[path = "core/segmented_stage.rs"]
mod segmented_stage;
#[path = "core/server_copy.rs"]
mod server_copy;
#[path = "core/session.rs"]
mod session;
#[path = "core/stage_evidence.rs"]
mod stage_evidence;
#[path = "core/stage_finish.rs"]
mod stage_finish;
#[path = "core/io.rs"]
mod streams;
#[path = "core/tree_remove.rs"]
mod tree_remove;
#[path = "core/url.rs"]
mod url;
#[path = "core/wire.rs"]
mod wire;

#[cfg(test)]
#[path = "core/abgleich_task_batch_time_tests.rs"]
mod abgleich_task_batch_time_tests;
#[cfg(test)]
#[path = "core/batch_fake.rs"]
mod batch_fake;
#[cfg(test)]
#[path = "core/batch_tests.rs"]
mod batch_tests;
#[cfg(test)]
#[path = "core/delete_speed_task_tests.rs"]
mod delete_speed_task_tests;
#[cfg(test)]
#[path = "core/range_tests.rs"]
mod range_tests;
#[cfg(test)]
#[path = "core/segmented_stage_fake.rs"]
mod segmented_stage_fake;
#[cfg(test)]
#[path = "core/segmented_stage_tests.rs"]
mod segmented_stage_tests;
#[cfg(test)]
#[path = "core/server_copy_task_tests.rs"]
mod server_copy_task_tests;
#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use backend::SmbBackend;
pub use errors::names_missing_share;
pub use session::SmbConfig;
pub use url::{backend_from_url, root_has_share, root_with_share};

#[path = "core/capacity.rs"]
mod capacity;

#[path = "core/folder_reservation.rs"]
mod folder_reservation;
