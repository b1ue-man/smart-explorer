//! Content search through the storage interface ("Suchen in Dateien", plan
//! docs/plaene/2026-11-27-dateien-suche-ansicht section B2): where the files
//! lie a backend searches them itself (`Backend::content_search`: the SSH
//! agent, a Share host); everywhere else (FTP/FTPS, WebDAV, SMB, Google Drive,
//! SFTP without agent, older peers) the search reads the files through the
//! backend (`list_dir` + `open_read`), several at once, with the same filters,
//! matching and events. Links are never entered or read.
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;

use crate::agent_proto::{
    search_local_tree, search_tree, ContentEvent, ContentQuery, ContentSummary, ContentTree,
    TreeEntry, TreeKind,
};

use super::{Backend, VfsMeta, VfsResult};

/// Files a search that reads through a backend opens at once.
const READ_THREADS: usize = 8;

/// The content search below `root` of `backend`: the backend's own search
/// where it offers one, else reading its files. `Err` for an invalid query
/// (`InvalidInput`, German text), an unlistable root, a cancel
/// (`Interrupted`), a closed receiver or a failure of the backend's search.
pub fn content_search(
    backend: &dyn Backend,
    root: &str,
    query: &ContentQuery,
    tx: Sender<ContentEvent>,
    cancel: &AtomicBool,
) -> VfsResult<ContentSummary> {
    // An invalid pattern fails here alike for every backend, before any event.
    crate::agent_proto::prepare_content_search(query)?;
    if let Some(summary) = backend.content_search(root, query, tx.clone(), cancel)? {
        return Ok(summary);
    }
    read_search(backend, root, query, &tx, cancel)
}

/// The content search of the local folder (or file) `native_root` on every
/// core: what `LocalBackend::content_search` answers with.
pub fn local_content_search(
    native_root: &Path,
    query: &ContentQuery,
    tx: Sender<ContentEvent>,
    cancel: &AtomicBool,
) -> VfsResult<Option<ContentSummary>> {
    let emit = |event| tx.send(event).is_ok();
    search_local_tree(native_root, query, &emit, cancel).map(Some)
}

/// The search that reads the files through `backend`.
fn read_search(
    backend: &dyn Backend,
    root: &str,
    query: &ContentQuery,
    tx: &Sender<ContentEvent>,
    cancel: &AtomicBool,
) -> VfsResult<ContentSummary> {
    let tree = BackendTree { backend, root };
    let threads = backend.parallelism().clamp(1, READ_THREADS);
    let emit = |event| tx.send(event).is_ok();
    search_tree(&tree, query, threads, &emit, cancel)
}

struct BackendTree<'b> {
    backend: &'b dyn Backend,
    root: &'b str,
}

impl BackendTree<'_> {
    fn path(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.to_string()
        } else if self.root.ends_with('/') {
            format!("{}{rel}", self.root)
        } else {
            format!("{}/{rel}", self.root)
        }
    }
}

impl ContentTree for BackendTree<'_> {
    /// A root whose status cannot be read (a peer's export list) is taken as
    /// a folder: its listing reports the failure.
    fn root_file(&self) -> io::Result<Option<TreeEntry>> {
        let Ok(meta) = self.backend.stat(self.root) else {
            return Ok(None);
        };
        if meta.is_dir || meta.is_symlink || meta.special {
            return Ok(None);
        }
        let mut entry = entry(meta);
        if entry.name.is_empty() {
            entry.name = self.root.rsplit('/').next().unwrap_or_default().to_string();
        }
        Ok(Some(entry))
    }

    fn list(&self, rel: &str) -> io::Result<Vec<TreeEntry>> {
        let listing = self.backend.list_dir(&self.path(rel))?;
        Ok(listing
            .into_iter()
            .map(|meta| match super::validate_child_name(&meta.name) {
                Ok(()) => entry(meta),
                Err(error) => {
                    let mut refused = entry(meta);
                    refused.kind = TreeKind::Unreadable(error.to_string());
                    refused
                }
            })
            .collect())
    }

    fn open(&self, rel: &str, entry: &TreeEntry) -> io::Result<Box<dyn Read + '_>> {
        let reader: Box<dyn Read + '_> = self
            .backend
            .open_read_id(&self.path(rel), entry.id.as_deref())?;
        Ok(reader)
    }
}

/// A listed entry: links and special files are never entered or read; hidden
/// as the backend reports it or, where it cannot (SFTP, FTP, WebDAV), a
/// leading dot.
fn entry(meta: VfsMeta) -> TreeEntry {
    let kind = if meta.is_symlink || meta.special {
        TreeKind::Other
    } else if meta.is_dir {
        TreeKind::Folder
    } else {
        TreeKind::File
    };
    TreeEntry {
        hidden: meta.hidden || meta.name.starts_with('.'),
        name: meta.name,
        kind,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        id: meta.id,
    }
}

#[cfg(test)]
#[path = "content_read_tests.rs"]
mod tests;
