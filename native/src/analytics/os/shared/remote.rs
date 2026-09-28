use super::{Progress, ScanOutcome, ScanPhase};
use crate::vfs::Backend;
use std::sync::atomic::Ordering;

/// The same remote selection boundary is used by the GUI and exporting workers.
pub fn scan_remote(backend: &dyn Backend, root: &str, progress: &Progress) -> ScanOutcome {
    let segment = progress.remote_segment();
    let progress = &segment;
    let before = progress.snapshot();
    progress.set_phase(ScanPhase::Preparing, root);
    let result = backend.scan_storage(root, progress);
    match result {
        Ok(Some(outcome)) => return outcome,
        Err(_) if progress.check_cancel().is_err() => return ScanOutcome::canceled(),
        Err(error) => return ScanOutcome::failed(root, error.to_string()),
        Ok(None) => {}
    }
    if !backend.supports_walk_tree() {
        return super::scan_backend(backend, root, progress);
    }
    progress.set_phase(ScanPhase::Legacy, root);
    let on_progress = |files, bytes| {
        progress.files.store(before.files.saturating_add(files), Ordering::Relaxed);
        progress.bytes.store(before.bytes.saturating_add(bytes), Ordering::Relaxed);
        progress.check_cancel().is_ok()
    };
    match backend.walk_tree(root, &on_progress) {
        Ok(Some(tree)) if progress.check_cancel().is_ok() => {
            let mut outcome = ScanOutcome::complete(super::from_wire(tree));
            if progress.snapshot().phase == ScanPhase::Legacy {
                outcome.notes.push("Die Gegenstelle nutzt den älteren Analysepfad; für den lokalen Worker und vollständige Fortschrittsmeldungen beide Geräte aktualisieren.".into());
            }
            outcome
        }
        _ if progress.check_cancel().is_err() => ScanOutcome::canceled(),
        Ok(None) => super::scan_backend(backend, root, progress),
        Err(error) => ScanOutcome::failed(root, error.to_string()),
        Ok(Some(_)) => ScanOutcome::canceled(),
    }
}
