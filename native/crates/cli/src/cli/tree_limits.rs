//! What the destination of a copy or move can store (U7, U11): names its
//! filesystem allows, the largest file it holds (FAT: 4 GiB − 1) and which
//! spellings it treats as one name (`a.txt`/`A.txt` on FAT and exFAT). A
//! plan leaves out every entry the destination refuses, with a German reason
//! per entry, and copies the others, as robocopy and rsync do; nothing is
//! written first only to fail after up to 4 GiB.
use crate::cli::entry_kind::EntryKind;
use crate::cli::tree_guard::join;
use crate::cli::tree_observer::TreeObserver;
use crate::vfs::{Backend, NameRules, Scheme, TargetLimits, VfsMeta};
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// Deepest folder the scan of `needs_plan` enters; deeper trees are left to
/// the plan, which reports its own limit.
const MAX_SCAN_DEPTH: usize = 512;

/// The storage rules of one location (destination or source of a copy).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::cli) struct TargetRules {
    pub(in crate::cli) limits: TargetLimits,
    pub(in crate::cli) names: NameRules,
}

impl TargetRules {
    /// The rules below `path` of `backend`: the backend's target limits;
    /// name folding and the filesystem's name only for local paths (remote
    /// protocols do not report them).
    pub(in crate::cli) fn of(backend: &dyn Backend, path: &str) -> Self {
        let limits = crate::vfs::target_limits(backend, path);
        let names = if backend.scheme() == Scheme::Local {
            crate::vfs::local_name_rules(path)
        } else {
            NameRules::default()
        };
        Self { limits, names }
    }

    /// Why an entry named `name` with `meta` cannot be stored here (German),
    /// or `None`. Links and special files are never written, so never refused.
    pub(in crate::cli) fn refusal(&self, name: &str, meta: &VfsMeta) -> Option<String> {
        if meta.is_symlink || (meta.special && !meta.is_dir) {
            return None;
        }
        if let Some(issue) = self.limits.name_issue(name) {
            return Some(self.names.name_reason(issue, self.limits.max_name));
        }
        match self.limits.max_file_size {
            Some(limit) if !meta.is_dir && meta.size > limit => Some(self.names.size_reason(limit)),
            _ => None,
        }
    }

    /// The entries of one listed folder (by index) this destination refuses,
    /// each with its reason: names and sizes it cannot store, and every name
    /// it treats as the same as another one. Of such spellings the first in
    /// byte order is kept, so a run decides the same way every time.
    pub(in crate::cli) fn refusals(&self, children: &[VfsMeta]) -> HashMap<usize, String> {
        let mut refused = HashMap::new();
        let mut order: Vec<usize> = Vec::with_capacity(children.len());
        for (index, child) in children.iter().enumerate() {
            match self.refusal(&child.name, child) {
                Some(reason) => {
                    refused.insert(index, reason);
                }
                None if EntryKind::of(child).is_omitted_in_trees() => {}
                None => order.push(index),
            }
        }
        if !self.names.folding.folds() {
            return refused;
        }
        order.sort_by(|left, right| children[*left].name.cmp(&children[*right].name));
        let mut kept: HashMap<String, usize> = HashMap::with_capacity(order.len());
        for index in order {
            let Some(key) = self.names.folding.key(&children[index].name) else {
                continue;
            };
            match kept.entry(key) {
                Entry::Occupied(first) => {
                    let reason = self
                        .names
                        .fold_reason(&children[index].name, &children[*first.get()].name);
                    refused.insert(index, reason);
                }
                Entry::Vacant(slot) => {
                    slot.insert(index);
                }
            }
        }
        refused
    }

    /// Whether a source with `source` rules can hold an entry this
    /// destination refuses: a size limit, naming rules or a name length the
    /// source does not share, or name folding the source does not do. A
    /// Windows NTFS source to NTFS, or ext4 to ext4, never can.
    pub(in crate::cli) fn may_refuse_from(&self, source: &TargetRules) -> bool {
        let target = &self.limits;
        target.max_file_size.is_some()
            || (target.windows_names && !source.limits.windows_names)
            || (target.max_name.is_some() && target.max_name != source.limits.max_name)
            || (self.names.folding.folds() && !source.names.folding.folds())
    }
}

/// Whether the tree below `root` holds an entry `rules` refuses or one the
/// listing cannot name (so only the plan, which reports each, may copy it).
/// A listing that fails answers `true` as well: the plan reports it.
pub(in crate::cli) fn needs_plan(
    backend: &dyn Backend,
    root: &str,
    rules: &TargetRules,
    observer: &TreeObserver,
) -> Result<bool, String> {
    let mut pending = vec![(root.to_string(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        observer.check()?;
        if depth > MAX_SCAN_DEPTH {
            return Ok(true);
        }
        let Ok(listing) = crate::cli::tree_guard::listing(backend, &directory) else {
            return Ok(true);
        };
        if !listing.omitted.is_empty() || !rules.refusals(&listing.entries).is_empty() {
            return Ok(true);
        }
        for child in listing.entries {
            if EntryKind::of(&child) == EntryKind::Dir {
                pending.push((join(&directory, &child.name), depth + 1));
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "tree_limits_tests.rs"]
mod tests;
