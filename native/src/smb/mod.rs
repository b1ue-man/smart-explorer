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
//! `ReplaceIfExists`.

#[path = "core/backend.rs"]
mod backend;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/listing.rs"]
mod listing;
#[path = "core/replace.rs"]
mod replace;
#[path = "core/session.rs"]
mod session;
#[path = "core/io.rs"]
mod streams;
#[path = "core/url.rs"]
mod url;
#[path = "core/wire.rs"]
mod wire;

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;

pub use backend::SmbBackend;
pub use errors::names_missing_share;
pub use session::SmbConfig;
pub use url::{backend_from_url, root_has_share, root_with_share};
