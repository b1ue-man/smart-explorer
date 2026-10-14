//! Folder deletion over FTP without commands per entry beyond the DELE
//! itself. `vfs::remove_tree_listed` lists every folder once and stats only
//! the folder asked for; the listing here keeps names and types (MLSD facts
//! or LIST rows) and skips the SIZE + MDTM that a browsing LIST spends on each
//! regular file (metadata.rs). DELE goes one after another on the browsing
//! connection: a control connection answers one command at a time, and more
//! connections would each cost a login of their own and end in congestion at
//! the server's connection limit (pool.rs).
//!
//! DELE never deletes a folder and RMD only an empty one (the server's
//! unlink/rmdir), so nothing beyond the listed tree goes. A link listed as a
//! link (MLSD `type=OS.unix=symlink`, `slink` or `link`, a LIST `l` row) is
//! deleted as the link. A server that reports a link with its target's type
//! is entered here just as by the generic walk, whose stat sees that type.
use super::core_impl::{parse_list_line_unchecked, FtpBackend};
use super::errors::{command_path, map, reply_code};
use super::io_adapters::FtpConnection;
use super::metadata::parse_mlsx;
use crate::vfs::{
    Backend, DeleteTarget, RecursiveDeleteFailure, RecursiveDeleteProgress, RecursiveDeleteReport,
    Scheme, VfsMeta, VfsResult,
};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn remove_tree(
    backend: &FtpBackend,
    path: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(RecursiveDeleteProgress),
) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
    crate::vfs::remove_tree_listed(&TypesOnly(backend), path, cancel, progress, |leaves| {
        // One DELE per round trip: a cancel takes effect before the next one.
        remove_in_order(leaves, |leaf| {
            if cancel.load(Ordering::Acquire) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            backend.remove_file(&leaf.path)
        })
    })
}

/// One DELE after another, one result per leaf in leaf order. After the
/// first failure nothing more is sent; the remaining leaves get its kind and
/// text.
pub(super) fn remove_in_order(
    leaves: &[DeleteTarget],
    mut remove: impl FnMut(&DeleteTarget) -> io::Result<()>,
) -> Vec<io::Result<()>> {
    let mut failure: Option<(io::ErrorKind, String)> = None;
    leaves
        .iter()
        .map(|leaf| {
            if let Some((kind, text)) = &failure {
                return Err(io::Error::new(*kind, text.clone()));
            }
            let result = remove(leaf);
            if let Err(error) = &result {
                failure = Some((error.kind(), error.to_string()));
            }
            result
        })
        .collect()
}

/// The backend as the tree walk sees it: listings with names and types
/// only, everything else unchanged.
struct TypesOnly<'a>(&'a FtpBackend);

impl Backend for TypesOnly<'_> {
    fn scheme(&self) -> Scheme {
        self.0.scheme()
    }
    fn root_display(&self) -> String {
        self.0.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        list_types(self.0.pool.primary(), path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.0.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
}

/// One folder's names and types: MLSD where offered, else LIST -a inside the
/// folder (plain LIST where refused, as browsing does), as metadata.rs lists.
fn list_types(connection: &FtpConnection, folder: &str) -> io::Result<Vec<VfsMeta>> {
    command_path(folder)?;
    let features = connection.features()?;
    let (mlsx, lines) = connection.with_stream_read(|stream| {
        if features.mlsd {
            match stream.mlsd(Some(folder)) {
                Ok(lines) => return Ok((true, lines)),
                Err(error) if matches!(reply_code(&error), Some(500 | 502 | 504)) => {}
                Err(error) => return Err(map(error)),
            }
        }
        // CWD keeps LIST from reading the folder name as options or a glob;
        // the previous folder is restored before the connection is shared.
        let original = stream.pwd().map_err(map)?;
        stream.cwd(folder).map_err(map)?;
        let listed = match stream.list(Some("-a")) {
            Err(error) if matches!(reply_code(&error), Some(500 | 501 | 502 | 504 | 550)) => {
                stream.list(None).map_err(map)
            }
            other => other.map_err(map),
        };
        stream.cwd(&original).map_err(map)?;
        listed.map(|lines| (false, lines))
    })?;
    if !mlsx {
        if let Ok(mut cached) = connection.features.lock() {
            if let Some(features) = cached.as_mut() {
                features.mlsd = false;
            }
        }
    }
    entries(lines, mlsx)
}

/// Names and types of one listing. A row this cannot classify (unreadable,
/// not addressable over the control connection, a repeated name) fails the
/// listing, so an unknown entry never passes for an absent one and its folder
/// is never emptied.
pub(super) fn entries(lines: Vec<String>, mlsx: bool) -> io::Result<Vec<VfsMeta>> {
    let mut names = HashSet::new();
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if !mlsx && line.starts_with("total ") {
            continue;
        }
        let entry = if mlsx {
            match parse_mlsx(&line, None)? {
                Some(entry) => entry,
                None => continue,
            }
        } else {
            parse_list_line_unchecked(&line)?
        };
        if matches!(entry.name.as_str(), "." | "..") {
            continue;
        }
        if entry.name.contains('\u{fffd}')
            || crate::vfs::validate_child_name(&entry.name).is_err()
            || command_path(&entry.name).is_err()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FTP child cannot be addressed by this path/control interface: {:?}",
                    entry.name
                ),
            ));
        }
        if !names.insert(entry.name.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FTP child names collide",
            ));
        }
        out.push(entry);
    }
    Ok(out)
}
