use super::tree_guard::validate_same_source;
use super::tree_plan::TransferPlan;
use crate::vfs::Backend;

pub(super) fn remove_transferred_source(
    backend: &dyn Backend,
    plan: &TransferPlan,
) -> Result<(), String> {
    // A move only removes the exact tree that was copied. A newly added child,
    // a changed identity, or a changed file aborts before the first deletion.
    plan.validate_source_tree(backend)?;
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
        if entry.source.is_dir {
            let remaining = backend.list_dir(&entry.source_path).map_err(|error| {
                format!(
                    "cannot verify moved source directory is empty: {}: {error}",
                    entry.source_path
                )
            })?;
            if !remaining.is_empty() {
                return Err(format!(
                    "moved source directory gained an unplanned child: {}",
                    entry.source_path
                ));
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
    Ok(())
}
