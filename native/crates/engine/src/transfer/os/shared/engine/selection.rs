//! Cancellable folder preview. It never mutates either endpoint.
use crate::transfer::{Endpoint, FolderConflictPreview, JobItems, Layout, TransferJob};
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "selection_count.rs"]
mod count;
pub use count::count_folder_conflict;

fn backend<'a>(endpoint: &'a Endpoint, local: &'a LocalBackend) -> &'a dyn Backend {
    match endpoint {
        Endpoint::Local => local,
        Endpoint::Remote(handle) => &**handle,
    }
}

pub fn folder_conflicts(
    job: &TransferJob,
    cancel: &AtomicBool,
) -> io::Result<Vec<FolderConflictPreview>> {
    if job.resume.is_some() || job.layout == Layout::Flatten {
        return Ok(Vec::new());
    }
    let JobItems::Roots { paths, base } = &job.items else {
        return Ok(Vec::new());
    };
    let local = LocalBackend::new("/");
    let source = backend(&job.source, &local);
    let target = backend(&job.target, &local);
    let mut out = Vec::new();
    let gate = job
        .source
        .is_local()
        .then(|| crate::transfer::AccessGate::new(&super::JobView::of(job).gate_root()));
    for path in paths {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let local = job.source.is_local();
        let inspect =
            || crate::source_media::with_cancellation(cancel, || root_kind(source, local, path));
        let mut inspected = inspect();
        if inspected
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
        {
            if let Some(gate) = &gate {
                if gate.request() == crate::transfer::AccessAnswer::Granted {
                    inspected = inspect();
                }
            }
        }
        let (is_dir, is_link) = inspected?;
        if !is_dir || is_link || job.folders.roots.contains_key(path) {
            continue;
        }
        let relative = super::super::engine_names::root_rel(path, base.as_deref());
        let destination = super::super::engine_names::join_rel(&job.target_dir, &relative);
        let existing =
            match crate::source_media::with_cancellation(cancel, || target.stat(&destination)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
                Ok(_) => true,
            };
        let keep_both_supported = !target.has_duplicate_file_names()
            && crate::vfs::directory_reservation_supported(target, &job.target_dir);
        if !existing && keep_both_supported {
            continue;
        }
        let parent = super::super::engine_names::parent_rel(&relative).unwrap_or("");
        let name = super::super::engine_names::base_name(&relative);
        let mut keep_both = String::new();
        for index in 2..=crate::vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS {
            if !keep_both_supported {
                break;
            }
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let candidate =
                super::super::engine_names::join_rel(parent, &format!("{name} ({index})"));
            let path = super::super::engine_names::join_rel(&job.target_dir, &candidate);
            match crate::source_media::with_cancellation(cancel, || target.stat(&path)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    keep_both = candidate;
                    break;
                }
                Err(error) => return Err(error),
                Ok(_) => {}
            }
        }
        out.push(FolderConflictPreview {
            source: path.clone(),
            relative,
            keep_both,
            keep_both_supported,
            existing,
            ..Default::default()
        });
    }
    Ok(out)
}

/// Whether the selected root is a folder and whether it is a link (M12): a
/// local root costs one `lstat` and no name lookup; a remote one its
/// backend's `stat`.
fn root_kind(source: &dyn Backend, local: bool, path: &str) -> io::Result<(bool, bool)> {
    if local {
        let native = super::super::walk_listers::native(path);
        let metadata = crate::local_access::symlink_metadata(&native)?;
        let link = crate::local_access::metadata_is_link_like(&native, &metadata);
        return Ok((metadata.is_dir(), link));
    }
    source.stat(path).map(|meta| (meta.is_dir, meta.is_symlink))
}

fn preview_context_valid(
    state: &std::sync::Mutex<crate::transfer::folder_conflict::FolderPreviewState>,
    context: Option<&super::record_io::RecordContext>,
) -> bool {
    if context.is_none_or(|context| crate::vault::check_transfer_blob_context_live(context).is_ok())
    {
        return true;
    }
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.folders.clear();
    state.ready = true;
    state.error = Some(
        "Der aktuelle Tresorzustand konnte nicht bestätigt werden. Den Auftrag erneut prüfen."
            .to_string(),
    );
    false
}

pub struct FolderPreviewScan {
    state: std::sync::Arc<std::sync::Mutex<crate::transfer::folder_conflict::FolderPreviewState>>,
    cancel: std::sync::Arc<AtomicBool>,
}

/// Calls the GUI's wake-up when dropped: every end of the preview thread,
/// including an error, is seen at once (M12).
struct WakeOnDrop(Box<dyn Fn() + Send>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

impl FolderPreviewScan {
    pub fn start(job: TransferJob) -> io::Result<Self> {
        Self::start_with_wake(job, Box::new(|| {}))
    }

    /// `start`; `wake` is called when the folders are known, when their
    /// counts are complete and when the preview ends, so a GUI shows the
    /// result at once instead of at its next timer (M12).
    pub fn start_with_wake(job: TransferJob, wake: Box<dyn Fn() + Send>) -> io::Result<Self> {
        let state = std::sync::Arc::new(std::sync::Mutex::new(
            crate::transfer::folder_conflict::FolderPreviewState::default(),
        ));
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let progress = state.clone();
        let stop = cancel.clone();
        std::thread::Builder::new()
            .name("folder-conflict-preview".into())
            .spawn(move || {
                let wake = WakeOnDrop(wake);
                let paths: Vec<&str> = match &job.items {
                    JobItems::Roots { paths, .. } => paths.iter().map(String::as_str).collect(),
                    JobItems::Pairs(pairs) => {
                        pairs.iter().map(|pair| pair.source.as_str()).collect()
                    }
                };
                let context = match super::record_io::record_locations(
                    super::Side::of(&job.source),
                    super::Side::of(&job.target),
                    &paths,
                    &job.target_dir,
                )
                .and_then(|locations| super::record_io::record_context(&locations))
                {
                    Ok(context) => context,
                    Err(_) => {
                        let mut state = progress
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.error =
                            Some("Der Tresorzustand des Auftrags ist unbestätigt".to_string());
                        state.ready = true;
                        return;
                    }
                };
                let mut folders = match folder_conflicts(&job, &stop) {
                    Ok(folders) => folders,
                    Err(error) => {
                        if !preview_context_valid(&progress, context.as_ref()) {
                            return;
                        }
                        let mut state = progress
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state.error = Some(error.to_string());
                        state.ready = true;
                        return;
                    }
                };
                if !preview_context_valid(&progress, context.as_ref()) {
                    return;
                }
                {
                    let mut state = progress
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state.folders = folders.clone();
                    state.ready = true;
                }
                (wake.0)();
                for (index, folder) in folders.iter_mut().enumerate() {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    count_folder_conflict(&job, folder, &stop, &|folder| {
                        if !preview_context_valid(&progress, context.as_ref()) {
                            stop.store(true, Ordering::Release);
                            return;
                        }
                        let mut state = progress
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(current) = state.folders.get_mut(index) {
                            *current = folder.clone();
                        }
                    });
                }
                (wake.0)();
                while context.is_some() && !stop.load(Ordering::Acquire) {
                    if !preview_context_valid(&progress, context.as_ref()) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            })?;
        Ok(Self { state, cancel })
    }

    pub fn snapshot(&self) -> crate::transfer::folder_conflict::FolderPreviewState {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for FolderPreviewScan {
    fn drop(&mut self) {
        self.cancel();
    }
}
