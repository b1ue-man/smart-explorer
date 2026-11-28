//! A content search of a local folder (the app's local search, the SSH agent's
//! `ContentSearch`): the directory listing without following links (Windows
//! junctions and symbolic links count as links, data reparse points stay
//! files and folders), hidden as the host shows it (Windows: the attribute;
//! elsewhere a leading dot), files opened without following a link, and the
//! kernel's pseudo file systems (`/proc`, `/sys`, `/dev`, `/run`) left out.
//! With `respect_ignore_files` the ignore files of the folders above the
//! root apply as well, as ripgrep reads them (`content_ignore.rs`).
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::content_query::{ContentQuery, ContentSummary};
use super::content_run::ContentEmit;
use super::content_walk::{
    local_parents, search_tree, ContentTree, ParentIgnore, TreeEntry, TreeKind,
};
use super::fs::{is_pseudo_dir, systemtime_ms};
use super::session::{emit, Sink};
use super::Frame;

struct LocalTree {
    root: PathBuf,
}

impl LocalTree {
    fn path(&self, rel: &str) -> PathBuf {
        let mut path = self.root.clone();
        for part in rel.split('/').filter(|part| !part.is_empty()) {
            path.push(part);
        }
        path
    }
}

impl ContentTree for LocalTree {
    fn root_file(&self) -> io::Result<Option<TreeEntry>> {
        let metadata = std::fs::metadata(&self.root)?;
        if metadata.is_dir() {
            return Ok(None);
        }
        let name = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Some(entry(name, &self.root, &metadata)))
    }

    fn list(&self, rel: &str) -> io::Result<Vec<TreeEntry>> {
        let folder = self.path(rel);
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&folder)? {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    entries.push(unreadable(String::new(), error.to_string()));
                    continue;
                }
            };
            let name = match item.file_name().into_string() {
                Ok(name) => name,
                Err(raw) => {
                    let reason = "Der Name ist kein gültiges UTF-8.".to_string();
                    entries.push(unreadable(raw.to_string_lossy().into_owned(), reason));
                    continue;
                }
            };
            let path = item.path();
            let metadata = match item.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    entries.push(unreadable(name, error.to_string()));
                    continue;
                }
            };
            let listed = entry(name, &path, &metadata);
            if listed.kind == TreeKind::Folder && is_pseudo_dir(&path.to_string_lossy()) {
                continue;
            }
            entries.push(listed);
        }
        Ok(entries)
    }

    fn open(&self, rel: &str, _entry: &TreeEntry) -> io::Result<Box<dyn Read + '_>> {
        let file = super::local_platform::open_regular_no_follow(&self.path(rel), false)?;
        Ok(Box::new(file))
    }

    fn parent_ignores(&self) -> Vec<ParentIgnore> {
        local_parents(&self.root)
    }
}

fn entry(name: String, path: &Path, metadata: &std::fs::Metadata) -> TreeEntry {
    let (link, special) = super::local_platform::metadata_class(path, metadata);
    let kind = if link || special {
        TreeKind::Other
    } else if metadata.is_dir() {
        TreeKind::Folder
    } else if metadata.is_file() {
        TreeKind::File
    } else {
        TreeKind::Other
    };
    TreeEntry {
        hidden: super::walk_keyed::local_hidden(&name, metadata),
        name,
        kind,
        size: metadata.len(),
        mtime_ms: metadata.modified().map(systemtime_ms).unwrap_or(0),
        id: None,
    }
}

fn unreadable(name: String, reason: String) -> TreeEntry {
    TreeEntry {
        name,
        kind: TreeKind::Unreadable(reason),
        hidden: false,
        size: 0,
        mtime_ms: 0,
        id: None,
    }
}

/// Searches the local folder (or file) `root` on every core of the host.
pub fn search_local_tree(
    root: &Path,
    query: &ContentQuery,
    emit: ContentEmit<'_>,
    cancel: &AtomicBool,
) -> io::Result<ContentSummary> {
    let tree = LocalTree {
        root: root.to_path_buf(),
    };
    search_tree(
        &tree,
        query,
        super::ordered_walk::walk_threads(),
        emit,
        cancel,
    )
}

/// The agent's answer to `ContentSearch`: `ContentEvent`s (hits and skips
/// charged against the request's credit), then `ContentDone`. The agent's
/// root confinement (`--serve-root`, Landlock) applies as for every request.
pub fn handle_content_search(
    sink: &Sink,
    id: u64,
    root: &str,
    query: &ContentQuery,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let send = |event| emit(sink, id, &Frame::ContentEvent(event)).is_ok();
    let summary = search_local_tree(Path::new(root), query, &send, cancel)?;
    emit(sink, id, &Frame::ContentDone(summary))
}
