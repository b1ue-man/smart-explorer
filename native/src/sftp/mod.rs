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
//! A range never fails for lack of a channel: it shares one (at most
//! `min(open-handles, 64)` handles each), waits for a lease to end or, with
//! no channel at all, goes over the main session (channel_pool.rs,
//! main_range.rs). On a long fat link one TCP stream caps the rate, so a
//! backend whose login needed no input (`SftpBackend::connect_with_origin`)
//! or went through the SSH agent (G3-10, agent_grant.rs) adds extra SSH
//! connections for transfers, one at a time while the measured throughput
//! still rises (judged half a second windows after the new one carried its
//! first byte); they only run the subsystem `sftp`, must show the main
//! connection's host key, close after a minute idle and are blocked for 15
//! minutes after any failure (security answer E13-S15 as changed by G3-10;
//! extra_link.rs, link_rules.rs, pool_growth.rs, pool_extra.rs).
//! A folder is deleted with one READDIR per folder and many REMOVEs in
//! flight on the main session (tree_remove.rs).
//! An interactive terminal (pty-req, then shell or exec) opens as one more
//! session channel and is driven like a command (terminal.rs, command.rs).

#[path = "core/agent_grant.rs"]
mod agent_grant;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/batch_get.rs"]
mod batch_get;
#[path = "core/batch_handoff.rs"]
mod batch_handoff;
#[path = "core/batch_ops.rs"]
mod batch_ops;
#[path = "core/batch_put.rs"]
mod batch_put;
#[path = "core/batch_put_evidence.rs"]
mod batch_put_evidence;
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
#[path = "core/enqueue.rs"]
mod enqueue;
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
#[path = "core/link_log.rs"]
mod link_log;
#[path = "core/link_rules.rs"]
mod link_rules;
#[path = "core/main_range.rs"]
mod main_range;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/pipelined_read.rs"]
mod pipelined_read;
#[path = "core/pool_channel.rs"]
mod pool_channel;
#[path = "core/pool_extra.rs"]
mod pool_extra;
#[path = "core/pool_growth.rs"]
mod pool_growth;
#[path = "core/pool_reader.rs"]
mod pool_reader;
#[path = "core/pool_writer.rs"]
mod pool_writer;
#[path = "core/posix_rename.rs"]
mod posix_rename;
#[path = "core/promote_evidence.rs"]
mod promote_evidence;
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
#[path = "core/stage_evidence.rs"]
mod stage_evidence;
#[path = "core/stage_finish.rs"]
mod stage_finish;
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
#[path = "core/extra_link_tests.rs"]
mod extra_link_tests;
#[cfg(test)]
#[path = "core/link_tests.rs"]
mod link_tests;
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
pub use config::{PasswordSource, SftpAuth, SftpConfig, SftpLoginOrigin};
pub use host_key::UnknownHostKey;
pub use host_key_ops::{
    forget_host_key, probe_host_key, trust_host_key, unknown_host_key, PresentedHostKey,
};
pub use terminal::{SshTerminalError, SshTerminalRequest};
pub use url::backend_from_url;

pub(crate) use batch_handoff::{hand_off as hand_off_packet, Forward as PacketForward};
pub(crate) use errors::io_err;
pub(crate) use stage_evidence::{stage_destination as upload_stage_destination, WrittenStages};

#[path = "core/capacity.rs"]
mod capacity;

#[path = "core/folder_reservation.rs"]
mod folder_reservation;
