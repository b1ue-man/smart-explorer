use super::copy_commit::{CommitMode, StagedUpload};
use super::entries::TransferErrorLog;
use super::progress::send_transfer_progress;
use super::upload_plan::UploadPlan;
use super::upload_stream::UploadSource;
use crate::app::app_models::{TransferKind, TransferMsg, TransferProgress};
use std::path::Path;
use std::sync::atomic::AtomicBool;

fn upload_with_mode(
    backend: &dyn crate::vfs::Backend, source: &Path, destination: &str,
    mode: CommitMode, cancel: Option<&AtomicBool>, progress: impl FnMut(u64),
) -> Result<(), String> {
    super::cancel::check_optional(cancel)?;
    let mut source = UploadSource::open(source)?;
    let mut staged = StagedUpload::open(backend, destination, cancel)?;
    if let Err(error) = source.copy_to(staged.writer(), cancel, progress) {
        return Err(staged.failed(error));
    }
    staged.commit(mode, cancel, || source.verify())
}

/// Explicit save-back/overwrite operation. Clipboard copies use Create below.
pub(in crate::app) fn upload_file(
    backend: &dyn crate::vfs::Backend, source: &Path, destination: &str,
) -> Result<(), String> {
    upload_with_mode(backend, source, destination, CommitMode::Replace, None, |_| {})
}

pub(super) fn upload_file_progress(
    backend: &dyn crate::vfs::Backend, source: &Path, destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>, progress: &mut TransferProgress,
    last: &mut std::time::Instant, cancel: &AtomicBool,
) -> Result<(), String> {
    upload_with_mode(backend, source, destination, CommitMode::Create, Some(cancel), |bytes| {
        progress.bytes_done = progress.bytes_done.saturating_add(bytes);
        send_transfer_progress(tx, progress, last, false);
    })
}

pub(in crate::app) fn upload_paths_progress(
    backend: &dyn crate::vfs::Backend, paths: &[String], destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool,
) {
    let plan = super::upload_plan::collect_paths(backend, paths, destination, cancel);
    run_upload_plan(backend, plan, destination, tx, cancel);
}

pub(super) fn run_upload_plan(
    backend: &dyn crate::vfs::Backend, plan: Result<UploadPlan, String>, destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>, cancel: &AtomicBool,
) {
    let mut errors = TransferErrorLog::default();
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            if error != super::cancel::CANCELED_ERROR { errors.push(error); }
            let mut progress = TransferProgress::new(TransferKind::Upload, "Lade hoch", 0, 0);
            progress.errors = errors.total();
            super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
            return;
        }
    };
    let bytes_total = plan.files.iter().map(|file| file.size).fold(0u64, u64::saturating_add);
    let mut progress = TransferProgress::new(TransferKind::Upload, "Lade hoch", plan.files.len() as u64, bytes_total);
    let mut last = std::time::Instant::now();
    let start = last;
    send_transfer_progress(tx, &progress, &mut last, true);

    // PutTree has no create-only destination contract and may partially mutate
    // its final tree before failing. Clipboard uploads deliberately use the
    // protected per-file path, including for backends advertising bulk support.
    // mkdir_all creates ancestors itself; preserve collection order and avoid
    // an all-directory sort merely to suppress repeated filtered ancestors.
    let mut seen_dirs = std::collections::HashSet::new();
    for directory in &plan.dirs {
        if super::cancel::requested(cancel) { break; }
        if !seen_dirs.insert(directory.as_str()) { continue; }
        let path = super::rjoin(destination, directory);
        if let Err(error) = backend.mkdir_all(&path) {
            errors.push(format!("Zielordner „{path}“ anlegen: {error}"));
        }
    }
    for file in plan.files {
        if super::cancel::requested(cancel) { break; }
        let path = super::rjoin(destination, &file.rel);
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        progress.errors = errors.total();
        send_transfer_progress(tx, &progress, &mut last, true);
        match upload_file_progress(backend, &file.src, &path, tx, &mut progress, &mut last, cancel) {
            Ok(()) => progress.files_done = progress.files_done.saturating_add(1),
            Err(error) if error == super::cancel::CANCELED_ERROR => {}
            Err(error) => errors.push(format!("{}: {error}", file.rel)),
        }
        // A successfully promoted file must be counted even when cancel was
        // requested inside its final backend operation.
        if super::cancel::requested(cancel) { break; }
        progress.errors = errors.total();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.total();
    super::cancel::send_done(tx, progress, errors.into_displayed(), cancel);
}
