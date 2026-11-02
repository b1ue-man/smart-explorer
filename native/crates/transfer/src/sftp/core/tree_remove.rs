//! Folder deletion over SFTP in few round trips: `vfs::remove_tree_listed`
//! lists every folder once (READDIR carries each entry's type, so no entry is
//! stat'ed) and the REMOVEs of a portion go out together on the main session.
//! Every SFTP request carries an id and russh-sftp matches each answer to its
//! request by that id (`RawSftpSession::send`), so many REMOVEs wait for their
//! answers at once instead of one per round trip. Nothing is replayed.
//!
//! REMOVE never deletes a folder and RMDIR only an empty one (the server's
//! unlink(2)/rmdir(2)), so nothing beyond the listed tree goes. A link is
//! listed with its own type (OpenSSH's READDIR reports lstat attributes, the
//! same as the LSTAT behind `stat`) and is removed as the link.
use super::backend::SftpBackend;
use super::io_err;
use super::session::SSH_WINDOW;
use crate::vfs::{remove_leaves_pipelined, DeleteTarget, VfsResult};
use std::io;

/// One full READ: russh-sftp's `Config::max_packet_len` default, the request
/// cap of the transfer channels (channel_pool.rs `CLIENT_MAX_PACKET`).
const FULL_READ: u32 = 262_144;

/// REMOVEs in flight at once: the depth of a pipelined download, one SSH
/// window of full READs (16 MiB / 256 KiB = 64; pool_reader.rs
/// `window_depth`, docs/TRANSFER_ENGINE.md). A REMOVE and its answer are far
/// smaller than a READ answer, so the window never holds them up. The server
/// answers in order, so with the main session's 10-s answer deadline
/// (russh-sftp `Config` default) each REMOVE may take about 150 ms.
pub(super) const REMOVE_DEPTH: usize = (SSH_WINDOW / FULL_READ) as usize;

impl SftpBackend {
    /// REMOVE every leaf of one portion, up to `REMOVE_DEPTH` at once; one
    /// result per leaf, in leaf order. A failure is reported to the
    /// connection as `mutate_sftp` does.
    pub(super) fn remove_leaves(&self, leaves: &[DeleteTarget]) -> Vec<VfsResult<()>> {
        let generation = match self.connection.current() {
            Ok(generation) => generation,
            Err(error) => return all_failed(leaves.len(), &error),
        };
        let (connection, generation) = (&self.connection, &generation);
        self.rt.block_on(remove_leaves_pipelined(
            leaves,
            REMOVE_DEPTH,
            move |leaf| async move {
                generation
                    .sftp()
                    .remove_file(leaf.path.as_str())
                    .await
                    .map_err(|error| {
                        connection.note_sftp_error(generation, &error);
                        io_err(error)
                    })
            },
        ))
    }
}

/// No request could be sent: every leaf carries the reason.
fn all_failed(count: usize, error: &io::Error) -> Vec<VfsResult<()>> {
    (0..count)
        .map(|_| Err(io::Error::new(error.kind(), error.to_string())))
        .collect()
}
