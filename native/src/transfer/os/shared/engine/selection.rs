//! Cancellable folder preview. It never mutates either endpoint.
use crate::transfer::{Endpoint, FolderConflictPreview, JobItems, Layout, TransferJob};
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

fn backend<'a>(endpoint: &'a Endpoint, local: &'a LocalBackend) -> &'a dyn Backend {
    match endpoint { Endpoint::Local => local, Endpoint::Remote(handle) => &**handle }
}

pub fn folder_conflicts(job: &TransferJob, cancel: &AtomicBool) -> io::Result<Vec<FolderConflictPreview>> {
    if job.resume.is_some() || job.layout == Layout::Flatten { return Ok(Vec::new()); }
    let JobItems::Roots { paths, base } = &job.items else { return Ok(Vec::new()); };
    let local = LocalBackend::new("/");
    let source = backend(&job.source, &local);
    let target = backend(&job.target, &local);
    let mut out = Vec::new();
    let gate = job.source.is_local().then(|| crate::transfer::AccessGate::new(&super::JobView::of(job).gate_root()));
    for path in paths {
        if cancel.load(Ordering::Acquire) { break; }
        let mut inspected = crate::source_media::with_cancellation(cancel, || source.stat(path));
        if inspected.as_ref().is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied) {
            if let Some(gate) = &gate {
                if gate.request() == crate::transfer::AccessAnswer::Granted { inspected = crate::source_media::with_cancellation(cancel, || source.stat(path)); }
            }
        }
        let meta = inspected?;
        if !meta.is_dir || meta.is_symlink || job.folders.roots.contains_key(path) { continue; }
        let relative = super::super::engine_names::root_rel(path, base.as_deref());
        let destination = super::super::engine_names::join_rel(&job.target_dir, &relative);
        let existing = match crate::source_media::with_cancellation(cancel, || target.stat(&destination)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
            Ok(_) => true,
        };
        let keep_both_supported = !target.has_duplicate_file_names() && crate::vfs::directory_reservation_supported(target, &job.target_dir);
        if !existing && keep_both_supported { continue; }
        let parent = super::super::engine_names::parent_rel(&relative).unwrap_or("");
        let name = super::super::engine_names::base_name(&relative);
        let mut keep_both = String::new();
        for index in 2..=crate::vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS {
            if !keep_both_supported { break; }
            if cancel.load(Ordering::Acquire) { break; }
            let candidate = super::super::engine_names::join_rel(parent, &format!("{name} ({index})"));
            let path = super::super::engine_names::join_rel(&job.target_dir, &candidate);
            match crate::source_media::with_cancellation(cancel, || target.stat(&path)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => { keep_both = candidate; break; }
                Err(error) => return Err(error),
                Ok(_) => {}
            }
        }
        out.push(FolderConflictPreview { source: path.clone(), relative, keep_both, keep_both_supported, existing, ..Default::default() });
    }
    Ok(out)
}

pub fn count_folder_conflict(job: &TransferJob, preview: &mut FolderConflictPreview, cancel: &AtomicBool, report: &dyn Fn(&FolderConflictPreview)) {
    let local = LocalBackend::new("/");
    let source = backend(&job.source, &local);
    let target = backend(&job.target, &local);
    let filter = super::super::entries::compile_remote_filter(job.filter.clone());
    let allow_backslash = job.source.is_local() && job.target.is_local() && super::super::platform::backslash_is_name_char();
    let mut stack = vec![(preview.source.clone(), preview.relative.clone(), 0usize)];
    let mut budget = super::super::entries::TransferCollectionBudget::default();
    let mut last_report = std::time::Instant::now();
    let mut complete = true;
    'scan: while let Some((path, relative, depth)) = stack.pop() {
        if cancel.load(Ordering::Acquire) { return; }
        let listed = match crate::source_media::with_cancellation(cancel, || crate::vfs::list_dir_tolerant(source, &path)) {
            Ok(listed) => listed,
            Err(_) => { omitted(preview, &path); complete = false; continue; }
        };
        complete &= listed.omitted.is_empty();
        for omission in listed.omitted { omitted(preview, &super::super::engine_names::join_rel(&path, &omission.rel)); }
        for entry in listed.entries {
            if cancel.load(Ordering::Acquire) { return; }
            if super::super::engine_names::name_problem(&entry.name, allow_backslash).is_some() {
                omitted(preview, &super::super::engine_names::join_rel(&path, &entry.name)); complete = false; continue;
            }
            let child_source = super::super::engine_names::join_rel(&path, &entry.name);
            let child_relative = super::super::engine_names::join_rel(&relative, &entry.name);
            if let Err(error) = budget.record_node(depth + 1, &[&child_source, &child_relative, &entry.name]) {
                preview.note = Some(error); complete = false; break 'scan;
            }
            if entry.is_symlink || entry.special || crate::apptrash::excluded_name(&entry.name) {
                omitted(preview, &child_source); complete = false; continue;
            }
            if let Some(filter) = &filter {
                let row = super::super::entries::remote_file_entry(&child_source, &path, &entry, filter.depth_for(&child_source));
                if entry.is_dir {
                    if filter.allows_dir_descendants(&row) { stack.push((child_source, child_relative, depth + 1)); }
                    continue;
                }
                if !filter.matches(&row) { continue; }
            }
            if entry.is_dir { stack.push((child_source, child_relative, depth + 1)); continue; }
            let destination = super::super::engine_names::join_rel(&job.target_dir, &child_relative);
            match crate::source_media::with_cancellation(cancel, || target.stat(&destination)) {
                Ok(_) => {
                    preview.colliding_files += 1;
                    if preview.samples.len() < 5 { preview.samples.push(child_relative); }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => preview.new_files += 1,
                Err(_) => { omitted(preview, &child_source); complete = false; }
            }
            if last_report.elapsed() >= super::PROGRESS_INTERVAL { report(preview); last_report = std::time::Instant::now(); }
        }
        report(preview);
    }
    preview.complete = complete;
    preview.counted = true;
    report(preview);
}

fn omitted(preview: &mut FolderConflictPreview, path: &str) {
    preview.omitted += 1;
    if preview.omitted_paths.len() < 5 { preview.omitted_paths.push(path.to_string()); }
}

fn preview_context_valid(state: &std::sync::Mutex<crate::transfer::folder_conflict::FolderPreviewState>, context: Option<&super::record_io::RecordContext>) -> bool {
    if context.is_none_or(|context| crate::vault::check_transfer_blob_context_live(context).is_ok()) { return true; }
    let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.folders.clear(); state.ready = true; state.error = Some("Der aktuelle Tresorzustand konnte nicht bestätigt werden. Den Auftrag erneut prüfen.".to_string());
    false
}

pub struct FolderPreviewScan {
    state: std::sync::Arc<std::sync::Mutex<crate::transfer::folder_conflict::FolderPreviewState>>,
    cancel: std::sync::Arc<AtomicBool>,
}

impl FolderPreviewScan {
    pub fn start(job: TransferJob) -> io::Result<Self> {
        let state = std::sync::Arc::new(std::sync::Mutex::new(crate::transfer::folder_conflict::FolderPreviewState::default()));
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let progress = state.clone();
        let stop = cancel.clone();
        std::thread::Builder::new().name("folder-conflict-preview".into()).spawn(move || {
            let paths: Vec<&str> = match &job.items { JobItems::Roots { paths, .. } => paths.iter().map(String::as_str).collect(), JobItems::Pairs(pairs) => pairs.iter().map(|pair| pair.source.as_str()).collect() };
            let context = match super::record_io::record_locations(super::Side::of(&job.source), super::Side::of(&job.target), &paths, &job.target_dir).and_then(|locations| super::record_io::record_context(&locations)) {
                Ok(context) => context,
                Err(_) => {
                    let mut state = progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.error = Some("Der Tresorzustand des Auftrags ist unbestätigt".to_string()); state.ready = true;
                    return;
                }
            };
            let mut folders = match folder_conflicts(&job, &stop) {
                Ok(folders) => folders,
                Err(error) => {
                    if !preview_context_valid(&progress, context.as_ref()) { return; }
                    let mut state = progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.error = Some(error.to_string()); state.ready = true;
                    return;
                }
            };
            if !preview_context_valid(&progress, context.as_ref()) { return; }
            {
                let mut state = progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                state.folders = folders.clone(); state.ready = true;
            }
            for (index, folder) in folders.iter_mut().enumerate() {
                if stop.load(Ordering::Acquire) { return; }
                count_folder_conflict(&job, folder, &stop, &|folder| {
                    if !preview_context_valid(&progress, context.as_ref()) { stop.store(true, Ordering::Release); return; }
                    let mut state = progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(current) = state.folders.get_mut(index) { *current = folder.clone(); }
                });
            }
            while context.is_some() && !stop.load(Ordering::Acquire) {
                if !preview_context_valid(&progress, context.as_ref()) { return; }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })?;
        Ok(Self { state, cancel })
    }

    pub fn snapshot(&self) -> crate::transfer::folder_conflict::FolderPreviewState {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    pub fn cancel(&self) { self.cancel.store(true, Ordering::Release); }
}

impl Drop for FolderPreviewScan {
    fn drop(&mut self) { self.cancel(); }
}
