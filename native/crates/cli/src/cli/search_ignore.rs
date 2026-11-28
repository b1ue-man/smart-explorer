//! Ignore files of a name search (`se search --gitignore`, MCP `search`
//! `gitignore`): `.gitignore` in a Git work tree, `.ignore` and
//! `.git/info/exclude`, as ripgrep and the content search read them
//! (`agent_proto::IgnoreChain`). The files are read through the backend of
//! the target, folder by folder as hits arrive, so a search next to the data
//! (SSH agent, Share host), which knows no ignore files, keeps its speed: its
//! hits are checked here, and a hit below an ignored folder is left out. On
//! a local folder and over SFTP the folders above the searched one count as
//! well, as in the content search.
use std::cell::RefCell;
use std::collections::HashMap;

use crate::agent_proto::{
    read_ignore_file, IgnoreChain, IgnoreFiles, ParentIgnore, EXCLUDE_PARTS, GITIGNORE_FILE,
    GIT_ENTRY, IGNORE_FILE,
};
use crate::vfs::{Backend, Scheme};

pub(super) struct BackendIgnores<'b> {
    backend: &'b dyn Backend,
    root: String,
    /// The rules that apply in each folder (relative to the root) read so far.
    chains: RefCell<HashMap<String, IgnoreChain>>,
    /// Folders (relative to the root) found ignored or kept.
    folders: RefCell<HashMap<String, bool>>,
}

/// The ignore files of the backend folder `path`; unreadable = absent.
fn folder_files(backend: &dyn Backend, path: &str) -> IgnoreFiles {
    let read = |path: &str| {
        let meta = backend.stat(path).ok()?;
        if meta.is_dir || meta.is_symlink || meta.special {
            return None;
        }
        let mut reader = backend.open_read(path).ok()?;
        read_ignore_file(&mut reader)
    };
    let git = super::ops::join(path, GIT_ENTRY);
    let git_meta = backend.stat(&git).ok();
    let exclude = match &git_meta {
        Some(meta) if meta.is_dir && !meta.is_symlink => {
            let info = super::ops::join(&git, EXCLUDE_PARTS[0]);
            read(&super::ops::join(&info, EXCLUDE_PARTS[1]))
        }
        _ => None,
    };
    IgnoreFiles {
        ignore: read(&super::ops::join(path, IGNORE_FILE)),
        gitignore: read(&super::ops::join(path, GITIGNORE_FILE)),
        exclude,
        has_git: git_meta.is_some(),
    }
}

/// The folder above `path` and the name of `path` in it; `None` at a root
/// (`/`, `C:/`, or a path without separator).
fn split_parent(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let cut = trimmed.rfind(['/', '\\'])?;
    let name = trimmed[cut + 1..].to_string();
    if name.is_empty() {
        return None;
    }
    let mut parent = trimmed[..cut].to_string();
    if parent.is_empty() || parent.ends_with(':') {
        parent.push('/');
    }
    Some((parent, name))
}

/// The ignore files of the folders above `root`, nearest first.
fn parent_files(backend: &dyn Backend, root: &str) -> Vec<ParentIgnore> {
    let mut parents = Vec::new();
    let mut lead: Vec<String> = Vec::new();
    let mut current = root.to_string();
    while let Some((parent, name)) = split_parent(&current) {
        if parent == current {
            break;
        }
        lead.insert(0, name);
        let files = folder_files(backend, &parent);
        if !files.is_empty() {
            parents.push(ParentIgnore {
                lead: lead.join("/"),
                files,
            });
        }
        current = parent;
    }
    parents
}

fn parent_rel(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

impl<'b> BackendIgnores<'b> {
    pub(super) fn new(backend: &'b dyn Backend, root: &str) -> Self {
        let above = match backend.scheme() {
            Scheme::Local | Scheme::Sftp => parent_files(backend, root),
            _ => Vec::new(),
        };
        let mut chains = HashMap::new();
        chains.insert(
            String::new(),
            IgnoreChain::above_root(&above).child("", &folder_files(backend, root)),
        );
        Self {
            backend,
            root: root.to_string(),
            chains: RefCell::new(chains),
            folders: RefCell::new(HashMap::new()),
        }
    }

    /// The rules inside the folder `folder` (relative to the root).
    fn chain(&self, folder: &str) -> IgnoreChain {
        if let Some(chain) = self.chains.borrow().get(folder) {
            return chain.clone();
        }
        let above = self.chain(parent_rel(folder));
        let files = folder_files(self.backend, &super::ops::join(&self.root, folder));
        let chain = above.child(folder, &files);
        self.chains
            .borrow_mut()
            .insert(folder.to_string(), chain.clone());
        chain
    }

    /// Whether the folder `folder` or one above it is ignored.
    fn folder_ignored(&self, folder: &str) -> bool {
        if folder.is_empty() {
            return false;
        }
        if let Some(ignored) = self.folders.borrow().get(folder) {
            return *ignored;
        }
        let parent = parent_rel(folder);
        let ignored = self.folder_ignored(parent) || self.chain(parent).ignored(folder, true);
        self.folders
            .borrow_mut()
            .insert(folder.to_string(), ignored);
        ignored
    }

    /// Whether the entry `rel` (a folder with `is_dir`) is left out by the
    /// ignore files on its way.
    pub(super) fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        if is_dir {
            return self.folder_ignored(rel);
        }
        let parent = parent_rel(rel);
        self.folder_ignored(parent) || self.chain(parent).ignored(rel, false)
    }
}

#[cfg(test)]
mod tests {
    use super::split_parent;

    #[test]
    fn mcp_coding_task_search_ignore_walks_up_backend_paths() {
        let pair = |parent: &str, name: &str| Some((parent.to_string(), name.to_string()));
        assert_eq!(split_parent("/srv/repo/src"), pair("/srv/repo", "src"));
        assert_eq!(split_parent("/srv/"), pair("/", "srv"));
        assert_eq!(split_parent("C:/work"), pair("C:/", "work"));
        assert_eq!(split_parent("C:\\work\\a"), pair("C:\\work", "a"));
        assert_eq!(split_parent("/"), None);
        assert_eq!(split_parent("C:/"), None);
        assert_eq!(split_parent("relative"), None);
    }
}
