//! Ignore files of a host content search (`content_search_ignore_v1`): the
//! files a held folder holds, and those of the folders above the search
//! root that the session may read on this host. Every file is opened as a
//! regular file relative to a held folder after the right was checked again.
//! An exported connection (`host_content_search_remote.rs`) gets the flag
//! with its query and applies the ignore files below it itself.
use super::Shared;
use crate::{
    agent_proto::{
        read_ignore_file, IgnoreChain, IgnoreFiles, ParentIgnore, EXCLUDE_PARTS, GITIGNORE_FILE,
        GIT_ENTRY, IGNORE_FILE,
    },
    local_access::DirectoryHandle,
    share::storage_roots,
    vfs::Scheme,
};
use std::ffi::OsStr;

/// The ignore files of a held folder, opened as regular files relative to
/// it after the right was checked again; `.git/info/exclude` only below a
/// `.git` folder the host's handle check admits. Unreadable = absent.
pub(super) fn held_ignore_files(shared: &Shared<'_>, directory: &DirectoryHandle) -> IgnoreFiles {
    if shared.access.check_read().is_err() {
        return IgnoreFiles::default();
    }
    let read = |folder: &DirectoryHandle, name: &str| {
        let mut file = folder.open_regular_child(OsStr::new(name)).ok()?;
        read_ignore_file(&mut file)
    };
    let git_folder = directory
        .open_child(OsStr::new(GIT_ENTRY))
        .ok()
        .filter(|git| crate::share::fs::ensure_local_share_handle_allowed(git).is_ok());
    let exclude = git_folder
        .as_ref()
        .and_then(|git| git.open_child(OsStr::new(EXCLUDE_PARTS[0])).ok())
        .and_then(|info| read(&info, EXCLUDE_PARTS[1]));
    // A `.git` file marks a linked work tree or a submodule.
    let has_git =
        git_folder.is_some() || directory.open_regular_child(OsStr::new(GIT_ENTRY)).is_ok();
    IgnoreFiles {
        ignore: read(directory, IGNORE_FILE),
        gitignore: read(directory, GITIGNORE_FILE),
        exclude,
        has_git,
    }
}

/// The ignore rules of the folders above the search root `path`, nearest
/// first, as far as they are local folders of an export the session may read
/// (each one resolved through the session's rights and opened like the root;
/// the first that is not ends the way up).
pub(super) fn parent_ignores(shared: &Shared<'_>, path: &str) -> IgnoreChain {
    if !shared.query.respect_ignore_files {
        return IgnoreChain::default();
    }
    let Ok(parts) = crate::share::fs::split_clean(path) else {
        return IgnoreChain::default();
    };
    let lead_slash = if path.starts_with('/') { "/" } else { "" };
    let mut parents = Vec::new();
    for depth in (1..parts.len()).rev() {
        let above = &parts[..depth];
        if shared.access.is_dynamic() && above.len() == 1 && above[0] == "Verbindungen" {
            break;
        }
        let parent = format!("{lead_slash}{}", above.join("/"));
        let Ok(target) = shared.access.resolve(&parent) else {
            break;
        };
        if target.backend.scheme() != Scheme::Local {
            break;
        }
        let Ok(physical) = storage_roots::local_path(&target) else {
            break;
        };
        if storage_roots::is_private(&physical) {
            break;
        }
        let Ok(handle) = storage_roots::open_local(&target) else {
            break;
        };
        let files = held_ignore_files(shared, &handle);
        if !files.is_empty() {
            parents.push(ParentIgnore {
                lead: parts[depth..].join("/"),
                files,
            });
        }
    }
    IgnoreChain::above_root(&parents)
}
