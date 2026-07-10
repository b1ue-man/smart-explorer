use super::tree_guard::{validate_destination_state, DestinationState};
use super::tree_plan::TransferPlan;
use super::tree_spool::SourceSpool;
use crate::vfs::Backend;
use std::io::Write;

pub(super) fn apply_transfer(
    source: &dyn Backend,
    destination: &dyn Backend,
    mut plan: TransferPlan,
    spool: &SourceSpool,
) -> Result<TransferPlan, String> {
    // These are the final complete read-only checks. No destination path or
    // adjacent staging object has been created before both succeed.
    plan.validate_source_tree(source)?;
    plan.validate_destinations(destination)?;
    create_external_ancestors(source, destination, &mut plan)?;
    create_planned_directories(source, destination, &mut plan)?;

    for index in 0..plan.entries.len() {
        if plan.entries[index].source.is_dir {
            continue;
        }
        publish_file(source, destination, &plan, spool, index)?;
    }
    Ok(plan)
}

fn create_external_ancestors(
    source: &dyn Backend,
    destination: &dyn Backend,
    plan: &mut TransferPlan,
) -> Result<(), String> {
    for index in 0..plan.destination_ancestors.len() {
        let path = plan.destination_ancestors[index].path.clone();
        let state = plan.destination_ancestors[index].state.clone();
        validate_destination_state(destination, &path, &state, true)?;
        if !matches!(state, DestinationState::Missing) {
            continue;
        }
        plan.validate_source_entry(source, 0)?;
        plan.validate_destination_ancestry(destination, 0)?;
        destination
            .mkdir_all(&path)
            .map_err(|error| format!("cannot create destination directory {path}: {error}"))?;
        let created = require_plain_directory(destination, &path)?;
        plan.destination_ancestors[index].state = DestinationState::Directory(created);
        super::tree_destination::validate_ancestor_collision(plan, destination, index)?;
    }
    Ok(())
}

fn create_planned_directories(
    source: &dyn Backend,
    destination: &dyn Backend,
    plan: &mut TransferPlan,
) -> Result<(), String> {
    for index in 0..plan.entries.len() {
        if !plan.entries[index].source.is_dir {
            continue;
        }
        let path = plan.entries[index].destination_path.clone();
        let state = plan.entries[index].destination.clone();
        plan.validate_source_entry(source, index)?;
        plan.validate_destination_ancestry(destination, index)?;
        validate_destination_state(destination, &path, &state, true)?;
        if !matches!(state, DestinationState::Missing) {
            continue;
        }
        destination
            .mkdir_all(&path)
            .map_err(|error| format!("cannot create destination directory {path}: {error}"))?;
        let created = require_plain_directory(destination, &path)?;
        plan.entries[index].destination = DestinationState::Directory(created);
        plan.validate_destination_parent_collision(destination, index)?;
    }
    Ok(())
}

fn publish_file(
    source: &dyn Backend,
    destination: &dyn Backend,
    plan: &TransferPlan,
    spool: &SourceSpool,
    index: usize,
) -> Result<(), String> {
    let entry = &plan.entries[index];
    plan.validate_source_entry(source, index)?;
    plan.validate_destination_ancestry(destination, index)?;
    plan.validate_destination_parent_collision(destination, index)?;
    validate_destination_state(
        destination,
        &entry.destination_path,
        &entry.destination,
        false,
    )?;
    let staged = crate::vfs::unique_staging_path(destination, &entry.destination_path, "cli-tree")
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let mut writer = destination
            .open_write(&staged)
            .map_err(|error| format!("cannot stage destination file {staged}: {error}"))?;
        let copied = spool.copy_file_into(index, &mut writer)?;
        writer
            .flush()
            .map_err(|error| format!("cannot flush staged destination {staged}: {error}"))?;
        drop(writer);
        let staged_metadata = destination
            .stat(&staged)
            .map_err(|error| format!("cannot verify staged destination {staged}: {error}"))?;
        if staged_metadata.is_dir || staged_metadata.is_symlink || staged_metadata.size != copied {
            return Err(format!(
                "staged destination could not be verified: {staged}"
            ));
        }

        // Creating the stage and copying from the private spool can take time.
        // Repeat both source and destination guards immediately before the one
        // visible namespace mutation.
        plan.validate_source_entry(source, index)?;
        plan.validate_destination_ancestry(destination, index)?;
        plan.validate_destination_parent_collision(destination, index)?;
        validate_destination_state(
            destination,
            &entry.destination_path,
            &entry.destination,
            false,
        )?;
        let promotion = match entry.destination {
            DestinationState::Missing => {
                crate::vfs::promote_staged_create(destination, &staged, &entry.destination_path)
            }
            DestinationState::File(_) => {
                crate::vfs::promote_staged_replace(destination, &staged, &entry.destination_path)
            }
            DestinationState::Directory(_) => {
                return Err(format!(
                    "file destination changed into a directory: {}",
                    entry.destination_path
                ))
            }
        };
        promotion.map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = destination.remove_file(&staged);
    }
    result
}

fn require_plain_directory(
    backend: &dyn Backend,
    path: &str,
) -> Result<crate::vfs::VfsMeta, String> {
    let metadata = backend
        .stat(path)
        .map_err(|error| format!("cannot verify destination directory {path}: {error}"))?;
    if metadata.is_symlink || !metadata.is_dir {
        return Err(format!(
            "destination directory is not plain after creation: {path}"
        ));
    }
    Ok(metadata)
}
