use super::tree_guard::validate_same_source;
use super::tree_plan::TransferPlan;
use crate::vfs::Backend;
use std::collections::HashSet;

/// Removes the moved source tree after the copy was published. Links and
/// special files the copy omitted stay where they are, and so do the
/// directories that still hold them; their paths are returned, root first.
pub(super) fn remove_transferred_source(
    backend: &dyn Backend,
    plan: &TransferPlan,
) -> Result<Vec<String>, String> {
    remove_transferred_source_checked(backend, plan, &|_| Ok(()))
}

pub(super) fn remove_transferred_source_checked(backend: &dyn Backend, plan: &TransferPlan, confirm: &dyn Fn(usize) -> Result<(), String>) -> Result<Vec<String>, String> {
    // A move only removes the exact tree that was copied. A newly added child,
    // a changed identity, or a changed file aborts before the first deletion.
    plan.validate_source_tree(backend)?;
    let keep = directories_holding_omissions(plan);
    let mut kept = Vec::new();
    for index in (0..plan.entries.len()).rev() {
        let entry = &plan.entries[index];
        plan.validate_source_entry(backend, index)?;
        let current = backend.stat(&entry.source_path).map_err(|error| {
            format!(
                "source changed before move cleanup: {}: {error}",
                entry.source_path
            )
        })?;
        validate_same_source(&entry.source, &current, &entry.source_path)?;
        confirm(index)?;
        if entry.source.is_dir {
            let remaining = backend.list_dir(&entry.source_path).map_err(|error| {
                format!(
                    "cannot verify moved source directory is empty: {}: {error}",
                    entry.source_path
                )
            })?;
            // Only the omitted entries and kept subdirectories may remain.
            let allowed: HashSet<&str> = if keep[index] {
                entry
                    .omitted
                    .iter()
                    .map(|omitted| plan.omitted[*omitted].listed.name.as_str())
                    .chain(
                        entry
                            .children
                            .iter()
                            .filter(|child| keep[**child])
                            .map(|child| plan.entries[*child].source.name.as_str()),
                    )
                    .collect()
            } else {
                HashSet::new()
            };
            if remaining
                .iter()
                .any(|child| !allowed.contains(child.name.as_str()))
            {
                return Err(format!(
                    "moved source directory gained an unplanned child: {}",
                    entry.source_path
                ));
            }
            if keep[index] {
                kept.push(entry.source_path.clone());
                continue;
            }
            backend.remove_dir(&entry.source_path).map_err(|error| {
                format!(
                    "cannot remove moved source directory {}: {error}",
                    entry.source_path
                )
            })?;
        } else {
            backend
                .remove_file_id(&entry.source_path, entry.source.id.as_deref())
                .map_err(|error| {
                    format!(
                        "cannot remove moved source file {}: {error}",
                        entry.source_path
                    )
                })?;
        }
    }
    kept.reverse();
    Ok(kept)
}

/// Directories that hold an omitted entry themselves or below them. A child
/// always has a larger index than its parent.
fn directories_holding_omissions(plan: &TransferPlan) -> Vec<bool> {
    let mut keep = vec![false; plan.entries.len()];
    for index in (0..plan.entries.len()).rev() {
        let entry = &plan.entries[index];
        if !entry.omitted.is_empty() {
            keep[index] = true;
        }
        if keep[index] {
            if let Some(parent) = entry.parent {
                keep[parent] = true;
            }
        }
    }
    keep
}
