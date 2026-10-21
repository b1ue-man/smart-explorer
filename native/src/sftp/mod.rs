//! SFTP backend (`russh` + `russh-sftp`) implementing `vfs::Backend`.
//!
//! Auth: password (with keyboard-interactive for PAM servers), key file
//! (+ optional passphrase and OpenSSH user certificate) or the running SSH
//! agent (session_auth.rs). Host keys: pinned in the app data
//! `known_hosts_sftp.txt` or listed in OpenSSH's known_hosts; an unknown key
//! is never stored silently but reported with its fingerprint until a person
//! confirms it (host_key.rs, host_key_ops.rs; the decision per handshake in
//! host_key_handler.rs, which also refuses every channel the server opens on
//! its own). An `@revoked` entry in OpenSSH's files refuses even a pinned key.
//! New connections no longer accept `ssh-rsa` host keys with SHA-1
//! signatures; servers pinned earlier keep them.
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
//! Many small files move as packets on one pool channel, their requests
//! overlapping (batch_ops.rs, batch_put.rs, batch_get.rs). A large file is
//! read in ranges, each over a pool channel of its own with its own server
//! window (pipelined_read.rs `with_end`); a large upload is written in ranges
//! only where no other account can swap an entry of the stage's folders
//! (security answer S7, segmented_stage.rs).
//! A folder is deleted with one READDIR per folder and many REMOVEs in
//! flight on the main session (tree_remove.rs).
//! An interactive terminal (pty-req, then shell or exec) opens as one more
//! session channel and is driven like a command (terminal.rs, command.rs).

#[path = "core/backend.rs"]
mod backend;
#[path = "core/batch_get.rs"]
mod batch_get;
#[path = "core/batch_ops.rs"]
mod batch_ops;
#[path = "core/batch_put.rs"]
mod batch_put;
#[path = "core/channel_pool.rs"]
mod channel_pool;
#[path = "core/command.rs"]
mod command;
#[path = "core/config.rs"]
mod config;
#[path = "core/connection.rs"]
mod connection;
#[path = "core/copy_data.rs"]
mod copy_data;
#[path = "core/errors.rs"]
mod errors;
#[path = "core/exec.rs"]
mod exec;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/host_key.rs"]
mod host_key;
#[path = "core/host_key_ops.rs"]
mod host_key_ops;
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
#[path = "core/reversible_replace.rs"]
mod reversible_replace;
#[path = "core/segmented_stage.rs"]
mod segmented_stage;
#[path = "core/session.rs"]
mod session;
#[cfg(unix)]
#[path = "os/ssh_agent_unix.rs"]
mod ssh_agent;
#[cfg(windows)]
#[path = "os/ssh_agent_windows.rs"]
mod ssh_agent;
#[path = "core/terminal.rs"]
mod terminal;
#[path = "core/transfer_ops.rs"]
mod transfer_ops;
#[path = "core/tree_remove.rs"]
mod tree_remove;
#[path = "core/url.rs"]
mod url;

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
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;
#[cfg(test)]
#[path = "core/segmented_stage_tests.rs"]
mod segmented_stage_tests;
#[cfg(test)]
#[path = "core/stage_copy_task_tests.rs"]
mod stage_copy_task_tests;
#[cfg(test)]
#[path = "core/transfer_engine_task_tests.rs"]
mod transfer_engine_task_tests;

pub use backend::SftpBackend;
pub use command::{signal_number, SshCommand, SshCommandControl, SshCommandEnd, SshCommandEvent};
pub use config::{SftpAuth, SftpConfig};
pub use host_key::UnknownHostKey;
pub use host_key_ops::{
    forget_host_key, probe_host_key, trust_host_key, unknown_host_key, PresentedHostKey,
};
pub use terminal::{SshTerminalError, SshTerminalRequest};
pub use url::backend_from_url;

pub(crate) use errors::io_err;
