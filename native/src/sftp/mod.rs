//! SFTP backend (`russh` + `russh-sftp`) implementing `vfs::Backend`.
//!
//! Auth: username/password OR keyfile (+ optional passphrase). Host keys use
//! trust-on-first-use against the app data `known_hosts_sftp.txt`
//! (accept + persist on first sight, reject on later mismatch).
//!
//! Async↔sync bridge: a private multi-threaded tokio runtime owned by the
//! backend. A worker thread continuously drives russh's background connection
//! task, while each blocking `Backend` method runs `rt.block_on(...)`. File I/O
//! is adapted to `std::io::{Read,Write}` by `block_on`-ing the tokio async reads
//! in chunks (no `SyncIoBridge` — it conflicts with this model). This keeps
//! scanner / copy / UI fully synchronous; see docs/REMOTE_LAYER_PLAN.md §1,§3.

#[path = "core/backend.rs"]
mod backend;
#[path = "core/config.rs"]
mod config;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/io_adapters.rs"]
mod io_adapters;
#[path = "os/shared/known_hosts.rs"]
mod known_hosts;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/session.rs"]
mod session;
#[path = "core/url.rs"]
mod url;

pub use backend::SftpBackend;
pub use config::{SftpAuth, SftpConfig};
pub use url::backend_from_url;

pub(crate) use errors::io_err;
