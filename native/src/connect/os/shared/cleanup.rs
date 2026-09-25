//! Headless cleanup after a connection, Direct peer or room was removed:
//! favourites, per-folder preferences and mounts of the scope are removed,
//! sync jobs are the user's own work and only reported as orphaned.
use super::location_prefs::{favorites_path, load_dir_sort, save_dir_sort};
use super::removal_scope::{
    filter_dir_sort, filter_favorites, job_references_scope, CleanupReport, MountScope,
    RemovedEndpointScope,
};

/// Remove favourites, folder preferences and mounts of `scope` and report
/// sync jobs that still reference it. Never fails as a whole: each store
/// reports its own error and the others are still cleaned.
pub fn cleanup_removed_endpoint_state(scope: &RemovedEndpointScope) -> CleanupReport {
    let mut report = CleanupReport::default();
    if scope.prefixes.is_empty() {
        report.orphaned_sync_jobs = Vec::new();
    } else {
        match remove_favorites(scope) {
            Ok(removed) => report.favorites_removed = removed,
            Err(error) => report
                .file_errors
                .push(format!("Favoriten bereinigen: {error}")),
        }
        match remove_dir_sort(scope) {
            Ok(removed) => report.dir_sort_removed = removed,
            Err(error) => report
                .file_errors
                .push(format!("Ordner-Einstellungen bereinigen: {error}")),
        }
        report.orphaned_sync_jobs = orphaned_sync_jobs(scope);
    }
    match stop_mounts(scope) {
        Ok(stopped) => report.mounts_stopped = stopped,
        Err(error) => report.mount_error = Some(error),
    }
    report
}

fn remove_favorites(scope: &RemovedEndpointScope) -> std::io::Result<usize> {
    let path = favorites_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let (kept, removed) = filter_favorites(&text, scope);
    if removed > 0 {
        std::fs::write(&path, kept.join("\n"))?;
    }
    Ok(removed)
}

fn remove_dir_sort(scope: &RemovedEndpointScope) -> std::io::Result<usize> {
    let mut map = load_dir_sort();
    let removed = filter_dir_sort(&mut map, scope);
    if removed > 0 {
        save_dir_sort(&map)?;
    }
    Ok(removed)
}

fn stop_mounts(scope: &RemovedEndpointScope) -> Result<usize, String> {
    if scope.mounts == MountScope::None {
        return Ok(0);
    }
    let mounts = crate::daemon::list_mounts()?;
    let mut stopped = 0;
    let mut failures = Vec::new();
    for snapshot in mounts {
        if !scope.matches_mount(&snapshot.config.source) {
            continue;
        }
        match crate::daemon::stop_mount(snapshot.config.id.clone()) {
            Ok(_) => stopped += 1,
            Err(error) => failures.push(format!("{}: {error}", snapshot.config.label)),
        }
    }
    if failures.is_empty() {
        Ok(stopped)
    } else {
        Err(format!(
            "{stopped} getrennt, nicht getrennt: {}",
            failures.join(", ")
        ))
    }
}

fn orphaned_sync_jobs(scope: &RemovedEndpointScope) -> Vec<String> {
    let Ok(jobs) = crate::syncjobs::load() else {
        return Vec::new();
    };
    jobs.iter()
        .filter(|job| job_references_scope(job, scope))
        .map(|job| job.name.clone())
        .collect()
}
