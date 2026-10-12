use super::tree_apply::apply_transfer;
use super::tree_guard::DestinationState;
use super::tree_observer::{TreeObserver, TreePhase};
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
    copy_entry_with(
        source,
        source_path,
        destination,
        destination_path,
        &TreeRequest {
            recursive,
            force,
            expected_root: None,
            destination_check: None,
            observer: &TreeObserver::default(),
        },
    )
}

/// Options of one tree copy. `destination_check` sees the planned state of
/// the destination root before anything is read or written (a caller's
/// precondition such as "unchanged since I read it"); the pipeline itself
/// re-validates that state before publishing.
pub(super) struct TreeRequest<'a> {
    pub(super) recursive: bool,
    pub(super) force: bool,
    pub(super) expected_root: Option<&'a crate::vfs::VfsMeta>,
    pub(super) destination_check: Option<&'a dyn Fn(&DestinationState) -> Result<(), String>>,
    pub(super) observer: &'a TreeObserver,
}

pub(super) fn copy_entry_with(
    source: &dyn Backend,
    source_path: &str,
    destination: &dyn Backend,
    destination_path: &str,
    request: &TreeRequest<'_>,
) -> Result<TransferReceipt, String> {
    let observer = request.observer;
    observer.set_phase(TreePhase::Planning);
    let plan = TransferPlan::build(
        source,
        source_path,
        destination,
        destination_path,
        request.recursive,
        request.force,
        request.expected_root,
        observer,
    )?;
    if let (Some(check), Some(root)) = (request.destination_check, plan.entries.first()) {
        check(&root.destination)?;
    }
    let (files, directories, bytes) =
        plan.entries
            .iter()
            .fold((0u64, 0u64, 0u64), |totals, entry| {
                if entry.source.is_dir {
                    (totals.0, totals.1 + 1, totals.2)
                } else {
                    (totals.0 + 1, totals.1, totals.2 + entry.source.size)
                }
            });
    observer.planned(files, directories, bytes);
    observer.set_phase(TreePhase::Reading);
    let spool = SourceSpool::collect(source, &plan, observer)?;
    observer.set_phase(TreePhase::Writing);
    let plan = apply_transfer(source, destination, plan, &spool, observer)?;
    Ok(TransferReceipt { plan })
}

impl TransferReceipt {
    /// Destination path of the transferred root.
    pub(super) fn destination(&self) -> &str {
        self.plan
            .entries
            .first()
            .map(|entry| entry.destination_path.as_str())
            .unwrap_or_default()
    }
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
