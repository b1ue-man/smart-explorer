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
//!
//! File transfers use extra SFTP channels of the same SSH connection
//! (channel_pool.rs) with many READs/WRITEs of one file in flight
//! (pipelined_read.rs, pool_writer.rs); listing and metadata stay on the
//! main session, which also serves transfers when no extra channel opens.
//! Copies inside one server run on the server (`copy-data`, copy_data.rs).

#[path = "core/backend.rs"]
mod backend;
#[path = "core/channel_pool.rs"]
mod channel_pool;
#[path = "core/config.rs"]
mod config;
#[path = "core/connection.rs"]
mod connection;
#[path = "core/copy_data.rs"]
mod copy_data;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/reversible_replace.rs"]
mod reversible_replace;
#[path = "core/exec.rs"]
mod exec;
#[path = "core/io_adapters.rs"]
mod io_adapters;
#[path = "os/shared/known_hosts.rs"]
mod known_hosts;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/pipelined_read.rs"]
mod pipelined_read;
#[path = "core/pool_reader.rs"]
mod pool_reader;
#[path = "core/pool_writer.rs"]
mod pool_writer;
#[path = "core/posix_rename.rs"]
mod posix_rename;
#[path = "core/session.rs"]
mod session;
#[path = "core/transfer_ops.rs"]
mod transfer_ops;
#[path = "core/url.rs"]
mod url;

#[cfg(test)]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;
#[cfg(test)]
#[path = "core/stage_copy_task_tests.rs"]
mod stage_copy_task_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use backend::SftpBackend;
pub use config::{SftpAuth, SftpConfig};
pub use url::backend_from_url;

pub(crate) use errors::io_err;
