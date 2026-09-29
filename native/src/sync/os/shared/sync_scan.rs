//! One source folder of a mirror's copy pass. Its entries are checked in
//! listing order exactly like the serial pass did, and compared with ONE
//! listing of the destination folder instead of a `stat` per file (on FTP a
//! `stat` lists the parent folder, which made the serial pass quadratic). A
//! name the listing cannot answer for sure (listed twice, or only in another
//! letter case) is still looked up with a `stat`, so case-insensitive and
//! duplicate-name destinations behave as before; where a `stat` is cheap,
//! every listed file is confirmed by one (see `copy_pass`). A destination
//! folder is checked to be a plain folder right before it is listed, and
//! every listing or `stat` is repeated after overload (`under_permits`).
use super::imp::{join, rel_of, require_plain_directory};
use super::sync_pass::Pass;
use crate::bisync::sync_flows::PairSide;
use crate::bisync::sync_overload::under_permits;
use crate::vfs::VfsMeta;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io;

/// A source folder to list and the state of its destination counterpart.
pub(super) struct DirTask {
    pub(super) path: String,
    pub(super) rel: String,
    pub(super) depth: usize,
    pub(super) target: Target,
}

/// The destination folder of a `DirTask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// Seen as a plain folder (or the checked root): list and compare.
    Listed,
    /// Missing: create it once, then every child is missing too.
    Create,
    /// Missing in a dry run: nothing is created, every child is missing.
    Absent,
}

/// A file to copy, with what the listings said about both sides.
pub(super) struct FileTask {
    pub(super) source: String,
    pub(super) destination: String,
    pub(super) rel: String,
    pub(super) meta: VfsMeta,
    pub(super) expected: Option<VfsMeta>,
    /// The destination entry was listed but must be confirmed by a `stat`
    /// before anything is decided (see `copy_pass`); `expected` is unused.
    pub(super) confirm: bool,
}

/// What to do about one source file, given its destination counterpart.
pub(super) enum Decision {
    Copy(Option<VfsMeta>),
    Skip,
    /// A link at the destination: a protected omission.
    Omit,
    Fail(String),
}

/// The serial pass's rule: copy when the destination is missing, differs in
/// size or is older than the source.
pub(super) fn decide(meta: &VfsMeta, counterpart: io::Result<Option<VfsMeta>>) -> Decision {
    match counterpart {
        Err(error) => Decision::Fail(format!("inspect destination: {error}")),
        Ok(None) => Decision::Copy(None),
        Ok(Some(found)) if found.is_symlink => Decision::Omit,
        Ok(Some(found)) if found.is_dir => {
            Decision::Fail("destination is a directory or link-like entry".to_string())
        }
        Ok(Some(found)) if found.size != meta.size || meta.mtime_ms > found.mtime_ms => {
            Decision::Copy(Some(found))
        }
        Ok(Some(_)) => Decision::Skip,
    }
}

enum Listed {
    One(VfsMeta),
    Several,
}

/// The children of one destination folder.
#[derive(Default)]
struct Destination {
    entries: HashMap<String, Listed>,
    folded: HashSet<String>,
}

enum Counterpart {
    Missing,
    Present(VfsMeta),
    /// Listed twice or only in another letter case: ask the backend.
    Unsure,
}

impl Destination {
    fn of(listing: Vec<VfsMeta>) -> Self {
        let mut destination = Self::default();
        for meta in listing {
            destination.folded.insert(meta.name.to_lowercase());
            let name = meta.name.clone();
            let listed = if destination.entries.contains_key(&name) {
                Listed::Several
            } else {
                Listed::One(meta)
            };
            destination.entries.insert(name, listed);
        }
        destination
    }

    fn counterpart(&self, name: &str) -> Counterpart {
        match self.entries.get(name) {
            Some(Listed::One(meta)) => Counterpart::Present(meta.clone()),
            Some(Listed::Several) => Counterpart::Unsure,
            None if self.folded.contains(&name.to_lowercase()) => Counterpart::Unsure,
            None => Counterpart::Missing,
        }
    }
}

/// Lists `task`'s source folder and queues its subfolders and the files to
/// copy; every problem is recorded, never skipped silently.
pub(super) fn scan_directory(pass: &Pass, task: DirTask) {
    let Some(entries) = source_entries(pass, &task.path) else {
        return;
    };
    let destination_dir = join(pass.dst_root, &task.rel);
    let Some(destination) = destination_entries(pass, &task, &destination_dir) else {
        return;
    };
    let mut names = HashSet::new();
    for meta in entries {
        if pass.scan_halted() {
            return;
        }
        if let Err(error) = crate::vfs::validate_child_name(&meta.name) {
            pass.error(task.path.clone(), error.to_string());
            continue;
        }
        if !names.insert(meta.name.clone()) {
            pass.error(
                task.path.clone(),
                format!("backend returned duplicate child name: {:?}", meta.name),
            );
            continue;
        }
        let source_path = join(&task.path, &meta.name);
        if !pass.within_budget(&source_path, task.depth + 1) {
            return;
        }
        let rel = rel_of(&source_path, pass.src_root);
        let destination_path = join(pass.dst_root, &rel);
        // The app trash and other apps' private storage (Android) are never
        // mirrored: protected omissions, like links.
        if meta.is_symlink
            || crate::apptrash::excluded_name(&meta.name)
            || crate::apptrash::hidden_app_folders_in(&task.path)
        {
            pass.omit(&rel);
            continue;
        }
        let counterpart = match destination.counterpart(&meta.name) {
            Counterpart::Missing => Ok(None),
            Counterpart::Present(found) => Ok(Some(found)),
            Counterpart::Unsure => match probe(pass, &destination_path) {
                Some(result) => result,
                None => return,
            },
        };
        let go_on = if meta.is_dir {
            directory(pass, &task, source_path, rel, destination_path, counterpart);
            true
        } else {
            file(pass, source_path, meta, rel, destination_path, counterpart)
        };
        if !go_on {
            return;
        }
    }
}

/// The source folder is still a plain folder (a link swapped in after it was
/// queued is refused before listing), then its entries.
fn source_entries(pass: &Pass, dir: &str) -> Option<Vec<VfsMeta>> {
    let changed = Cell::new(false);
    let result = under_permits(
        pass.cancel,
        &pass.progress,
        || pass.listing_permit(PairSide::A),
        |_| {
            changed.set(false);
            if let Err(error) = require_plain_directory(pass.src, dir, false) {
                changed.set(true);
                return Err(error);
            }
            pass.src.list_dir(dir)
        },
    )?;
    match result {
        Ok(entries) => Some(entries),
        Err(error) if changed.get() => {
            pass.error(
                dir,
                format!("source directory changed before traversal: {error}"),
            );
            None
        }
        Err(error) => {
            pass.error(dir, error.to_string());
            None
        }
    }
}

fn destination_entries(pass: &Pass, task: &DirTask, dir: &str) -> Option<Destination> {
    match task.target {
        Target::Absent => Some(Destination::default()),
        Target::Listed => {
            // Still a plain folder, not a link swapped in since the parent
            // was listed: a listing would follow it and copies land outside.
            let listed = under_permits(
                pass.cancel,
                &pass.progress,
                || pass.listing_permit(PairSide::B),
                |_| {
                    require_plain_directory(pass.dst, dir, false)?;
                    pass.dst.list_dir(dir)
                },
            )?;
            match listed {
                Ok(entries) => Some(Destination::of(entries)),
                Err(error) => {
                    pass.error(dir, format!("inspect destination: {error}"));
                    None
                }
            }
        }
        Target::Create => {
            if let Err(error) = pass.ensure_folder(&task.rel) {
                if !pass.canceled() {
                    pass.error(
                        dir,
                        format!("create destination directory: {}", error.message),
                    );
                }
                return None;
            }
            // Created now, or meanwhile by someone else: a plain folder
            // either way, never a link that could redirect the copies.
            let checked = under_permits(
                pass.cancel,
                &pass.progress,
                || pass.listing_permit(PairSide::B),
                |_| require_plain_directory(pass.dst, dir, false),
            )?;
            if let Err(error) = checked {
                pass.error(dir, format!("create destination directory: {error}"));
                return None;
            }
            // New files are still published create-only after a final
            // `stat`, so content that appeared meanwhile is never replaced.
            Some(Destination::default())
        }
    }
}

/// The per-name `stat` of the serial pass, for names the destination
/// listing cannot answer for sure; `None` once canceled.
fn probe(pass: &Pass, path: &str) -> Option<io::Result<Option<VfsMeta>>> {
    under_permits(
        pass.cancel,
        &pass.progress,
        || pass.listing_permit(PairSide::B),
        |_| match pass.dst.stat(path) {
            Ok(meta) => Ok(Some(meta)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        },
    )
}

fn directory(
    pass: &Pass,
    parent: &DirTask,
    source_path: String,
    rel: String,
    destination_path: String,
    counterpart: io::Result<Option<VfsMeta>>,
) {
    let target = match counterpart {
        Err(error) => {
            pass.error(destination_path, error.to_string());
            return;
        }
        Ok(Some(found)) if found.is_symlink => {
            pass.omit(&rel);
            return;
        }
        Ok(Some(found)) if found.is_dir => Target::Listed,
        Ok(Some(_)) => {
            let message = format!(
                "create destination directory: directory root is link-like or not a directory: {destination_path}"
            );
            pass.error(destination_path, message);
            return;
        }
        Ok(None) if pass.dry_run => Target::Absent,
        Ok(None) => Target::Create,
    };
    pass.queue_dir(DirTask {
        path: source_path,
        rel,
        depth: parent.depth + 1,
        target,
    });
}

/// False when the pass ended while the file waited for queue space.
fn file(
    pass: &Pass,
    source_path: String,
    meta: VfsMeta,
    rel: String,
    destination_path: String,
    counterpart: io::Result<Option<VfsMeta>>,
) -> bool {
    let counterpart = match counterpart {
        Ok(Some(listed)) if pass.confirm_listing => {
            if !pass.dry_run && !pass.dst.is_local() {
                // A remote `stat` costs a round trip: the copy workers
                // confirm in parallel before they decide.
                return pass.queue_file(FileTask {
                    source: source_path,
                    destination: destination_path,
                    rel,
                    meta,
                    expected: Some(listed),
                    confirm: true,
                });
            }
            match probe(pass, &destination_path) {
                Some(confirmed) => confirmed,
                None => return false,
            }
        }
        other => other,
    };
    let expected = match decide(&meta, counterpart) {
        Decision::Copy(expected) => expected,
        Decision::Skip => {
            pass.skipped();
            return true;
        }
        Decision::Omit => {
            pass.omit(&rel);
            return true;
        }
        Decision::Fail(message) => {
            pass.error(destination_path, message);
            return true;
        }
    };
    if pass.dry_run {
        pass.would_copy(&destination_path);
        return true;
    }
    pass.queue_file(FileTask {
        source: source_path,
        destination: destination_path,
        rel,
        meta,
        expected,
        confirm: false,
    })
}
