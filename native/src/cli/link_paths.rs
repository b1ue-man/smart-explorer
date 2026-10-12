//! Links in the paths a copy or move names explicitly. Like `cp -H`: a link
//! named as source or destination is followed, a link inside a copied tree is
//! not (`tree_plan` reports and skips it).
//!
//! - Local paths: links in the destination's existing part (a linked home or
//!   folder, `/tmp -> /private/tmp`) and a destination that is a link to a
//!   folder lead where they point; a source link is copied as what it points
//!   to, under the link's name.
//! - Remotes have no way to resolve a link: a source link to a file is read
//!   the way `se cat` reads it; a link to a folder and a destination path
//!   through a link are refused with the reason.
//! - A move never follows a source link: it renames the link itself.
use super::ops::parent_of;
use super::private_file::PrivateFile;
use super::target::Target;
use super::tree_observer::{TreeObserver, TreePhase};
use super::tree_ops::{TreeOutcome, TreeRequest};
use crate::vfs::VfsMeta;
use std::io::{self, Read, Write};

/// What `se cp` copies for its source argument.
pub(super) enum SourceRoot {
    /// Not a link: copied as named.
    Plain,
    /// A local link: the entry it leads to, copied under the link's name.
    Resolved { target: Target, name: String },
    /// A link on a remote: the file content it leads to.
    RemoteLink,
}

pub(super) fn source_root(src: &Target, meta: &VfsMeta) -> Result<SourceRoot, String> {
    if !meta.is_symlink {
        return Ok(SourceRoot::Plain);
    }
    if !src.backend.is_local() {
        return Ok(SourceRoot::RemoteLink);
    }
    let resolved = crate::transfer::canonical_folder(&super::os::local_path(&src.path))
        .map_err(|error| dangling(&src.path, &error))?;
    Ok(SourceRoot::Resolved {
        target: src.at_path(slash_path(&resolved)),
        name: meta.name.clone(),
    })
}

/// The local destination with the links in its existing part resolved, or
/// `None` when no link is involved (the path is used as given).
pub(super) fn resolve_destination(dst: &Target) -> Result<Option<Target>, String> {
    if !dst.backend.is_local() {
        return Ok(None);
    }
    let leaf_is_link = match dst.backend.stat(&dst.path) {
        Ok(meta) => meta.is_symlink,
        Err(_) => false,
    };
    let mut through_link = leaf_is_link;
    let mut probe = parent_of(&dst.path);
    while !through_link {
        let Some(path) = probe else { break };
        through_link = dst.backend.stat(&path).is_ok_and(|meta| meta.is_symlink);
        probe = parent_of(&path).filter(|parent| parent != &path);
    }
    if !through_link {
        return Ok(None);
    }
    let native = super::os::local_path(&dst.path);
    if leaf_is_link {
        let leads_to = std::fs::metadata(&native).map_err(|error| dangling(&dst.path, &error))?;
        if !leads_to.is_dir() {
            let real = crate::transfer::canonical_folder(&native)
                .map(|path| slash_path(&path))
                .unwrap_or_default();
            return Err(format!(
                "destination {} is a link to a file; Smart Explorer does not write through links to files. Name the file it points to instead: {real}",
                dst.path
            ));
        }
    }
    let resolved = resolve_existing_part(&native).map_err(|error| {
        format!(
            "cannot resolve the links in destination {}: {error}",
            dst.path
        )
    })?;
    Ok(Some(dst.at_path(resolved)))
}

/// The path with its existing part resolved; missing names below it are
/// appended unchanged (they are created as plain folders later).
fn resolve_existing_part(native: &std::path::Path) -> io::Result<String> {
    let absolute = if native.is_absolute() {
        native.to_path_buf()
    } else {
        std::env::current_dir()?.join(native)
    };
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.try_exists()? {
        let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no existing parent folder",
            ));
        };
        missing.push(name.to_os_string());
        existing = parent;
    }
    let mut resolved = crate::transfer::canonical_folder(existing)?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(slash_path(&resolved))
}

/// Copies the file a remote link points to (`se cp @host:/link dst`).
pub(super) fn copy_remote_link(
    src: &Target,
    dst: &Target,
    dst_path: &str,
    force: bool,
    observer: &TreeObserver,
) -> Result<TreeOutcome, String> {
    observer.set_phase(TreePhase::Reading);
    let (private, file) = PrivateFile::create("smart-explorer-link-copy-")?;
    if let Err(error) = read_link_content(src, file, observer) {
        observer.check()?;
        if src.backend.list_dir(&src.path).is_ok() {
            return Err(directory_link(&src.path));
        }
        return Err(format!(
            "cannot read the file the link {} points to: {error}",
            src.path
        ));
    }
    let receipt = private.publish(
        &*dst.backend,
        dst_path,
        &TreeRequest {
            recursive: false,
            force,
            expected_root: None,
            destination_check: None,
            observer,
        },
    )?;
    observer.set_phase(TreePhase::Done);
    Ok(TreeOutcome::complete(receipt.destination().to_string()))
}

fn read_link_content(
    src: &Target,
    mut file: std::fs::File,
    observer: &TreeObserver,
) -> Result<(), String> {
    let mut reader = src
        .backend
        .open_read(&src.path)
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        observer.check()?;
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        };
        file.write_all(&buffer[..read])
            .map_err(|error| format!("cannot write the private staging file: {error}"))?;
    }
    file.flush()
        .map_err(|error| format!("cannot write the private staging file: {error}"))
}

/// `se mv` of a link that cannot be renamed in place.
pub(super) fn link_move_refused(path: &str) -> String {
    format!(
        "{path} is a link; a link moves only by renaming it on the same connection, Smart Explorer does not recreate links elsewhere. Copy it (`se cp`) to store the content it points to."
    )
}

fn directory_link(path: &str) -> String {
    format!(
        "{path} is a link to a directory on a remote; Smart Explorer follows directory links only on local paths. Copy the directory it points to instead."
    )
}

fn dangling(path: &str, error: &io::Error) -> String {
    format!("{path} is a link whose target does not exist or cannot be reached: {error}")
}

fn slash_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
