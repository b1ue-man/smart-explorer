use super::tree_apply::apply_transfer;
use super::tree_plan::TransferPlan;
use super::tree_remove::remove_transferred_source;
use super::tree_spool::SourceSpool;
use crate::vfs::{Backend, DeleteTarget};

pub(super) struct TransferReceipt {
    plan: TransferPlan,
}

#[cfg(test)]
pub(super) fn copy_entry(
    source: &dyn Backend,
    source_path: &str,
    destination: &dyn Backend,
    destination_path: &str,
    recursive: bool,
    force: bool,
) -> Result<TransferReceipt, String> {
    copy_entry_from_snapshot(
        source,
        source_path,
        destination,
        destination_path,
        recursive,
        force,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_entry_from_snapshot(
    source: &dyn Backend,
    source_path: &str,
    destination: &dyn Backend,
    destination_path: &str,
    recursive: bool,
    force: bool,
    expected_root: Option<&crate::vfs::VfsMeta>,
) -> Result<TransferReceipt, String> {
    let plan = TransferPlan::build(
        source,
        source_path,
        destination,
        destination_path,
        recursive,
        force,
        expected_root,
    )?;
    let spool = SourceSpool::collect(source, &plan)?;
    let plan = apply_transfer(source, destination, plan, &spool)?;
    Ok(TransferReceipt { plan })
}

pub(super) fn remove_copied_source(
    backend: &dyn Backend,
    receipt: &TransferReceipt,
) -> Result<(), String> {
    remove_transferred_source(backend, &receipt.plan)
}

pub(super) fn remove_existing(
    backend: &dyn Backend,
    path: &str,
    recursive: bool,
) -> Result<(), String> {
    let metadata = backend.stat(path).map_err(|error| error.to_string())?;
    if metadata.is_dir && !metadata.is_symlink && !recursive {
        return Err(format!("{path} is a directory; pass --recursive"));
    }
    crate::vfs::remove_entry(
        backend,
        &DeleteTarget {
            path: path.to_string(),
            id: metadata.id,
            is_dir: metadata.is_dir,
            is_symlink: metadata.is_symlink,
        },
    )
    .map_err(|error| error.to_string())
}
