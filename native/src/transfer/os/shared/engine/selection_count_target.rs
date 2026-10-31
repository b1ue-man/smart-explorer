//! What the preview knows about the target folder of one source folder
//! (`selection_count.rs`, whose submodule this is): its names from one
//! tolerant listing, "absent", "unreadable", or "equal" when the folder's key
//! is the key of its source folder and nothing needs listing.
use super::super::super::update::{compare, Keys, Place};
use super::{Context, Node};
use crate::transfer::engine_names::join_rel;
use crate::transfer::{FolderConflictPreview, TransferJob};
use crate::vfs::{KeyTally, VfsMeta};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

/// The names of one target folder.
pub(super) enum Target {
    Absent,
    Unreadable,
    /// Not listed: the folder's key is equal to its source folder's.
    Equal,
    Listed {
        folded: bool,
        names: HashMap<String, Vec<VfsMeta>>,
        /// Names the target listed as omissions (folded like `names`).
        omitted: HashSet<String>,
    },
}

impl Target {
    pub(super) fn key(folded: bool, name: &str) -> String {
        if folded {
            name.to_lowercase()
        } else {
            name.to_string()
        }
    }

    pub(super) fn find(&self, name: &str) -> Option<&[VfsMeta]> {
        let Self::Listed { folded, names, .. } = self else {
            return None;
        };
        names.get(&Self::key(*folded, name)).map(Vec::as_slice)
    }

    /// The target lists `name` as an entry it cannot describe.
    pub(super) fn omits(&self, name: &str) -> bool {
        let Self::Listed {
            folded, omitted, ..
        } = self
        else {
            return false;
        };
        omitted.contains(&Self::key(*folded, name))
    }
}

/// The target folder of `node`: one listing.
pub(super) fn read_target(context: &Context<'_>, node: &Node) -> Target {
    if node.target_absent {
        return Target::Absent;
    }
    let dir = join_rel(context.target_dir, &node.relative);
    match crate::vfs::list_dir_tolerant(context.target, &dir) {
        Ok(listed) => {
            let folded = if context.target_local {
                cfg!(windows)
            } else {
                !context.target.case_sensitive_paths(&dir)
            };
            let omitted = listed
                .omitted
                .iter()
                .map(|omission| Target::key(folded, &omission.rel))
                .collect();
            let mut names: HashMap<String, Vec<VfsMeta>> =
                HashMap::with_capacity(listed.entries.len());
            for entry in listed.entries {
                names
                    .entry(Target::key(folded, &entry.name))
                    .or_default()
                    .push(entry);
            }
            Target::Listed {
                folded,
                names,
                omitted,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Target::Absent,
        Err(_) => Target::Unreadable,
    }
}

/// The folders with equal keys when the job applies the file rule and the
/// target is a folder that exists (`update_keys.rs`).
pub(super) fn equal_keys(
    job: &TransferJob,
    view: &super::super::super::JobView<'_>,
    preview: &FolderConflictPreview,
    cancel: &AtomicBool,
) -> Option<Keys> {
    if !preview.existing || !job.applies_update_rule() {
        return None;
    }
    let target_dir = join_rel(&job.target_dir, &preview.relative);
    let source = Place {
        side: view.source,
        root: &preview.source,
        flow: None,
    };
    let target = Place {
        side: view.target,
        root: &target_dir,
        flow: None,
    };
    compare(source, target, cancel)
}

/// What the folder of `node` holds when its key is the key of its target
/// folder and nothing in its subtree needs a look (`Keys::settled`): its
/// files count as equal and the folder is listed at neither side. Not for a
/// filtered job, whose count leaves files out.
pub(super) fn settled_by_key(context: &Context<'_>, node: &Node) -> Option<KeyTally> {
    if node.target_absent || context.filter.is_some() {
        return None;
    }
    context.keys.as_ref()?.settled(&node.inner, node.depth)
}

/// Whether the folder of `node` has the key of its target folder. A file with
/// an unknown time is never equal by key: the rule keeps both for it, which
/// only the listing of the target shows.
pub(super) fn equal_by_key(context: &Context<'_>, node: &Node, entries: &[VfsMeta]) -> bool {
    !node.target_absent
        && context
            .keys
            .as_ref()
            .is_some_and(|keys| keys.covers(&node.inner))
        && entries
            .iter()
            .all(|entry| entry.is_dir || entry.is_symlink || entry.special || entry.mtime_ms != 0)
}
