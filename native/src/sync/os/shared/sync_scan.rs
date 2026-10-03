//! Literal source/destination spellings meet under one NFC/case key.
use super::imp::require_plain_directory;
pub(super) use super::sync_compare::{decide, Decision};
use super::sync_compare::{file, Destination};
use super::sync_pass::Pass;
use crate::bisync::sync_flows::PairSide;
use crate::bisync::sync_overload::under_permits;
use crate::bisync::OmissionKind;
use crate::vfs::{VfsListing, VfsMeta};
use std::collections::{HashMap, HashSet};
use std::io;

pub(super) struct DirTask {
    pub(super) path: String,
    pub(super) rel: String,
    pub(super) target_rel: String,
    pub(super) depth: usize,
    pub(super) target: Target,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Listed,
    Create,
    Absent,
}
pub(super) struct FileTask {
    pub(super) source: String,
    pub(super) destination: String,
    pub(super) rel: String,
    pub(super) target_rel: String,
    pub(super) meta: VfsMeta,
    pub(super) expected: Option<VfsMeta>,
    pub(super) confirm: bool,
}

fn child(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Source and destination omitted children protect counterparts even when a
/// tolerant listing cannot return metadata for them.
fn omissions(pass: &Pass, listing: &VfsListing, rel: &str) {
    for omission in &listing.omitted {
        let relative = if crate::vfs::validate_child_name(&omission.rel).is_ok() {
            child(rel, &omission.rel)
        } else {
            rel.to_string()
        };
        pass.omit_kind(&relative, omission.reason.into());
    }
}

pub(super) fn scan_directory(pass: &Pass, task: DirTask) {
    if pass.scan_halted() {
        return;
    }
    let Some(listed) = under_permits(
        pass.cancel,
        &pass.progress,
        || pass.listing_permit(PairSide::A),
        |_| {
            if pass.canceled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "mirror stopped"));
            }
            crate::bisync::apply_boundary::guard(pass.src, pass.src_root, &task.rel, false)?;
            require_plain_directory(pass.src, &task.path, false)?;
            crate::vfs::list_dir_tolerant(pass.src, &task.path)
        },
    ) else {
        return;
    };
    let listing = match listed {
        Ok(listing) => listing,
        Err(error) => {
            pass.omit_kind(&task.rel, OmissionKind::Unreadable);
            pass.io_error(&task.path, error);
            return;
        }
    };
    omissions(pass, &listing, &task.rel);
    let destination_dir = match crate::vfs::sync_path(pass.dst, pass.dst_root, &task.target_rel) {
        Ok(path) => path,
        Err(error) => {
            pass.io_error(&task.path, error);
            return;
        }
    };
    let Some(destination) = destination_entries(pass, &task, &destination_dir) else {
        return;
    };
    let mut groups: HashMap<String, usize> = HashMap::new();
    for meta in &listing.entries {
        *groups
            .entry(pass.keys.key(&meta.name).into_owned())
            .or_default() += 1;
    }
    let mut seen = HashSet::new();
    for meta in listing.entries {
        if pass.scan_halted() {
            return;
        }
        let rel = child(&task.rel, &meta.name);
        if crate::vfs::validate_child_name(&meta.name).is_err() {
            pass.omit_kind(&task.rel, OmissionKind::NotRepresentable);
            continue;
        }
        let key = pass.keys.key(&meta.name).into_owned();
        if !seen.insert(key.clone()) {
            continue;
        }
        if groups.get(&key).copied().unwrap_or(0) != 1 {
            pass.omit_kind(&rel, OmissionKind::NameImpossibleOnTarget);
            continue;
        }
        if !pass.within_budget(&rel, task.depth + 1) {
            return;
        }
        let source_path = match crate::vfs::sync_child_path(pass.src, &task.path, &meta.name) {
            Ok(path) => path,
            Err(error) => {
                pass.io_error(&task.path, error);
                continue;
            }
        };
        match crate::bisync::snapshot_policy::protected(
            pass.src,
            pass.src_root,
            &source_path,
            &meta,
            false,
        ) {
            Ok(Some(kind)) => {
                pass.omit_kind(&rel, kind);
                continue;
            }
            Err(error) => {
                pass.omit_kind(&rel, OmissionKind::Unreadable);
                pass.io_error(&source_path, error);
                continue;
            }
            Ok(None) => {}
        }
        let found = match destination.entries.get(&key) {
            Some(None) => {
                pass.omit_kind(&rel, OmissionKind::NameImpossibleOnTarget);
                continue;
            }
            Some(Some(meta)) => Some(meta.clone()),
            None => None,
        };
        let target_name = found
            .as_ref()
            .map_or(meta.name.as_str(), |found| found.name.as_str());
        let target_rel = child(&task.target_rel, target_name);
        let destination_path =
            match crate::vfs::sync_child_path(pass.dst, &destination_dir, target_name) {
                Ok(path) => path,
                Err(error) => {
                    pass.io_error(&destination_dir, error);
                    continue;
                }
            };
        if let Some(found) = &found {
            match crate::bisync::snapshot_policy::protected(
                pass.dst,
                pass.dst_root,
                &destination_path,
                found,
                false,
            ) {
                Ok(Some(kind)) => {
                    pass.omit_kind(&rel, kind);
                    continue;
                }
                Err(error) => {
                    pass.omit_kind(&rel, OmissionKind::Unreadable);
                    pass.io_error(&destination_path, error);
                    continue;
                }
                Ok(None) => {}
            }
        }
        let protected = {
            let state = pass.lock();
            state.report.omissions.protects(&rel) || state.report.omissions.protects(&target_rel)
        };
        if protected {
            continue;
        }
        if let Err(error) = crate::bisync::apply_boundary::target(
            pass.dst,
            pass.dst_root,
            &target_rel,
            if meta.is_dir { None } else { Some(meta.size) },
        ) {
            if let Some(kind) = crate::bisync::apply_boundary::omitted(&error) {
                pass.omit_kind(&rel, kind);
            } else {
                pass.io_error(&destination_path, error);
            }
            continue;
        }
        if meta.is_dir {
            let target = match found {
                Some(found) if found.is_dir => Target::Listed,
                Some(_) => {
                    pass.error(&destination_path, "destination is not a directory");
                    continue;
                }
                None if pass.dry_run => Target::Absent,
                None => Target::Create,
            };
            pass.queue_dir(DirTask {
                path: source_path,
                rel,
                target_rel,
                depth: task.depth + 1,
                target,
            });
        } else if !file(
            pass,
            source_path,
            meta,
            rel,
            target_rel,
            destination_path,
            found,
        ) {
            return;
        }
    }
}

fn destination_entries(pass: &Pass, task: &DirTask, dir: &str) -> Option<Destination> {
    if task.target == Target::Absent {
        return Some(Destination::default());
    }
    let result = under_permits(
        pass.cancel,
        &pass.progress,
        || pass.listing_permit(PairSide::B),
        |_| {
            if pass.canceled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "mirror stopped"));
            }
            crate::bisync::apply_boundary::guard(pass.dst, pass.dst_root, &task.target_rel, false)?;
            if task.target == Target::Create {
                require_plain_directory(pass.dst, dir, true)?;
                if !crate::bisync::apply_stage::namespace(pass.dst, dir)? {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "new mirror directory is not namespace-durable",
                    ));
                }
            } else {
                require_plain_directory(pass.dst, dir, false)?;
            }
            crate::vfs::list_dir_tolerant(pass.dst, dir)
        },
    )?;
    match result {
        Ok(listing) => {
            omissions(pass, &listing, &task.target_rel);
            Some(Destination::of(listing.entries, pass.keys))
        }
        Err(error) => {
            pass.omit_kind(&task.rel, OmissionKind::Unreadable);
            pass.io_error(dir, error);
            None
        }
    }
}
