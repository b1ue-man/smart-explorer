//! Folder deletion over SMB in few round trips: `vfs::remove_tree_listed`
//! lists every folder once (QUERY_DIRECTORY carries each entry's attributes
//! and reparse tag, so no entry is opened for a stat) and the deletes of a
//! portion run side by side on clones of the session's connection. smb2
//! multiplexes clones over one session ("concurrent `execute` calls on
//! different clones multiplex over the same SMB session", smb2 `Connection`)
//! and points `Tree::delete_files` callers there to overlap server work.
//!
//! Each delete is the existing CREATE + SET_INFO + CLOSE compound on the
//! entry itself (`wire::delete`): a link, also one to a folder, goes as the
//! link and its target stays; a real folder is never deleted as a leaf and
//! only once it is empty (FileDispositionInformation). As `SmbSession::write`
//! a failure retires a dead connection and nothing is replayed.
use super::backend::SmbBackend;
use super::errors;
use super::url::entry_path;
use super::wire::{self, DeleteKind};
use crate::vfs::{remove_leaves_pipelined, DeleteTarget, VfsResult};
use std::io;

/// Sizes the batch by its credit charge: a delete is CREATE + SET_INFO +
/// CLOSE at one credit each (smb2 `CompoundOp::new`), the same three credits
/// `Connection::credit_capacity_for` counts for a CREATE + READ + CLOSE of up
/// to 64 KiB. Its answer is how many such chains the credit window funds at
/// once (512 / 3 = 170 before the server shows a lower ceiling); more would
/// only wait for credits. reader.rs sizes its READs the same way.
const CHAIN_PAYLOAD: u64 = 1;

impl SmbBackend {
    /// Delete every leaf of one portion below `share`, as many at once as the
    /// credit window funds; one result per leaf, in leaf order.
    pub(super) fn remove_tree_leaves(
        &self,
        share: &str,
        leaves: &[DeleteTarget],
    ) -> Vec<VfsResult<()>> {
        let (generation, tree) = match self.session.target(share) {
            Ok(target) => target,
            Err(error) => return all_failed(leaves.len(), &error),
        };
        let conn = generation.connection();
        let depth = conn.credit_capacity_for(CHAIN_PAYLOAD);
        let (conn, tree, generation) = (&conn, &tree, &generation);
        self.session.block_on(remove_leaves_pipelined(
            leaves,
            depth,
            move |leaf| async move {
                let target = entry_path(&leaf.path)?;
                if !target.share.eq_ignore_ascii_case(share) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "SMB-Eintrag {} liegt nicht in der Freigabe „{share}“",
                            leaf.path
                        ),
                    ));
                }
                wire::delete(conn, tree, &target.rel, DeleteKind::File)
                    .await
                    .map_err(|error| {
                        generation.note(&error);
                        errors::map(error, "Löschen", &leaf.path)
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
