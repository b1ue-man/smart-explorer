//! Content that does not come from a copyable source entry (MCP `write` and
//! `edit` text, the file a remote link points to) is first written to a
//! private local file and then published through the `se put` pipeline:
//! staged next to the destination, verified, published without exposing
//! partial content.
use super::tree_ops::{copy_entry_with, TransferReceipt, TreeRequest};
use std::fs::File;

pub(super) struct PrivateFile {
    // Removed with its content when the value is dropped.
    _directory: tempfile::TempDir,
    path: String,
}

impl PrivateFile {
    /// An empty private file, open for writing.
    pub(super) fn create(prefix: &str) -> Result<(Self, File), String> {
        let directory = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .map_err(|error| format!("cannot create a private staging file: {error}"))?;
        let path = directory.path().join("content");
        let file = File::create(&path)
            .map_err(|error| format!("cannot create a private staging file: {error}"))?;
        Ok((
            Self {
                path: path.to_string_lossy().replace('\\', "/"),
                _directory: directory,
            },
            file,
        ))
    }

    /// Publishes the file at `destination_path`. `request.recursive` is
    /// irrelevant: the source is one regular file.
    pub(super) fn publish(
        &self,
        destination: &dyn crate::vfs::Backend,
        destination_path: &str,
        request: &TreeRequest<'_>,
    ) -> Result<TransferReceipt, String> {
        copy_entry_with(
            &crate::vfs::LocalBackend::new("/"),
            &self.path,
            destination,
            destination_path,
            request,
        )
    }
}
