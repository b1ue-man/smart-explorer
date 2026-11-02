//! Host-local duplicate search, result remapping and bounded group portions.
use crate::analytics::{DuplicateReport, FinderLimits, FinderRoot, Progress, ReclaimProgress};
use crate::share::{
    fs_access::FsAccess,
    fs_response::{FsDuplicateGroup, FsDuplicateMessage, FsDuplicateSummary},
    storage_roots,
    wire::FsDuplicateSearch,
};
use std::{io, sync::atomic::Ordering};

pub(in crate::share) fn find(
    request: &FsDuplicateSearch,
    access: &FsAccess,
    progress: &ReclaimProgress,
) -> io::Result<DuplicateReport> {
    let mut p = Progress::default();
    p.cancel = progress.cancel.clone();
    let roots = storage_roots::resolve(&request.path, access, &p)?;
    progress.stage.map_directories(
        roots
            .roots
            .iter()
            .map(|root| {
                (
                    root.physical.as_ref().map_or_else(
                        || root.target.path.clone(),
                        |path| crate::local_access::display_path(path),
                    ),
                    root.visible.clone(),
                )
            })
            .collect(),
    );
    let local: Vec<FinderRoot> = roots
        .roots
        .iter()
        .filter(|root| root.retained)
        .filter_map(|root| root.physical.as_ref())
        .map(|path| FinderRoot {
            path: path.clone(),
            protected: crate::apptrash::ProtectedAreas::for_walk(path),
        })
        .collect();
    let mut report = crate::analytics::find_duplicates_in_roots_with_open(
        &local,
        progress,
        FinderLimits {
            min_bytes: request.min_bytes.max(1),
            candidate_text_bytes: crate::analytics::candidate_text_budget(),
            threads: crate::analytics::local_scan_threads(),
        },
        None,
        &storage_roots::excluded(),
        &|path| {
            roots
                .roots
                .iter()
                .find(|root| root.physical.as_deref() == Some(path))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Kandidatenwurzel ist nicht zugelassen",
                    )
                })
                .and_then(|root| storage_roots::open_local(&root.target))
        },
    );
    let mut root_failures = Vec::new();
    roots.plan.failures(&mut root_failures);
    if roots.roots.is_empty() {
        report.root_error = root_failures.first().cloned();
    }
    report.summary.errors.extend(root_failures);
    for group in &mut report.groups {
        for file in &mut group.items {
            file.path = visible(&roots, &file.path);
            file.name = file.path.rsplit('/').next().unwrap_or_default().into();
        }
    }
    for area in &mut report.summary.protected {
        area.area = visible(&roots, &area.area);
    }
    for error in &mut report.summary.errors {
        *error = replace(&roots, error);
    }
    report.root_error = report.root_error.map(|error| replace(&roots, &error));
    for root in roots
        .roots
        .iter()
        .filter(|root| root.retained && root.physical.is_none())
    {
        p.check_cancel()?;
        progress.stage.mirror(
            crate::analytics::ReclaimPhase::Walking,
            (0, 0, 0),
            &root.target.path,
        );
        let remote_progress = ReclaimProgress {
            cancel: progress.cancel.clone(),
            stage: progress.stage.clone(),
            ..Default::default()
        };
        let mut remote = crate::analytics::find_backend_duplicates(
            root.target.backend.clone(),
            &root.target.path,
            &remote_progress,
            request.min_bytes,
        );
        // Provider/agent MD5 is a candidate hint; only host-verified SHA-256
        // groups enter the Share wire's SHA-256 contract.
        remote.groups = crate::share::host_duplicate_verify::sha256_groups(
            &*root.target.backend,
            remote.groups,
            progress,
            &mut remote.summary.errors,
            &mut remote.summary.suppressed_errors,
        )?;
        remote.summary.groups = remote.groups.len() as u64;
        let scope = p.scoped(root.target.path.clone(), root.visible.clone());
        for group in &mut remote.groups {
            for file in &mut group.items {
                file.path = scope.visible_path(&file.path);
            }
        }
        for area in &mut remote.summary.protected {
            area.area = scope.visible_path(&area.area);
        }
        for error in &mut remote.summary.errors {
            *error = error.replace(&root.target.path, &root.visible);
        }
        let summary = remote.summary;
        report.summary.files = report.summary.files.saturating_add(summary.files);
        report.summary.bytes = report.summary.bytes.saturating_add(summary.bytes);
        report.summary.candidates = report.summary.candidates.saturating_add(summary.candidates);
        report.summary.compared = report.summary.compared.saturating_add(summary.compared);
        report.summary.protected.extend(summary.protected);
        let remaining = 64usize.saturating_sub(report.summary.errors.len());
        report.summary.suppressed_errors = report
            .summary
            .suppressed_errors
            .saturating_add(summary.errors.len().saturating_sub(remaining) as u64);
        report
            .summary
            .errors
            .extend(summary.errors.into_iter().take(remaining));
        report.summary.suppressed_errors = report
            .summary
            .suppressed_errors
            .saturating_add(summary.suppressed_errors);
        report.summary.limits.extend(summary.limits);
        if let Some(error) = remote.root_error {
            report
                .summary
                .errors
                .push(format!("{}: {error}", root.visible));
        }
        report.groups.extend(remote.groups);
    }
    if roots.roots.iter().filter(|root| root.retained).count() > 1
        && roots
            .roots
            .iter()
            .any(|root| root.retained && root.physical.is_none())
    {
        report.summary.limits.push("Verschiedene nichtlokale Backends werden jeweils auf ihrem Speichergerät verglichen; backendübergreifende Paare sind nicht erfasst".into());
    }
    report
        .groups
        .sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable));
    report.summary.groups = report.groups.len() as u64;
    if report.summary.errors.len() > 64 {
        report.summary.suppressed_errors = report
            .summary
            .suppressed_errors
            .saturating_add((report.summary.errors.len() - 64) as u64);
        report.summary.errors.truncate(64);
    }
    report.summary.limits.truncate(64);
    progress
        .files
        .store(report.summary.files, Ordering::Relaxed);
    progress
        .bytes
        .store(report.summary.bytes, Ordering::Relaxed);
    if progress.cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    Ok(report)
}

fn visible(roots: &storage_roots::Roots, path: &str) -> String {
    for root in roots.roots.iter().filter(|root| root.retained) {
        if let Some(physical) = &root.physical {
            let scope = Progress::default().scoped(
                crate::local_access::display_path(physical),
                root.visible.clone(),
            );
            let mapped = scope.visible_path(path);
            if mapped != root.visible
                || path.replace('\\', "/")
                    == crate::local_access::display_path(physical).replace('\\', "/")
            {
                return mapped;
            }
        }
    }
    "/".into()
}
fn replace(roots: &storage_roots::Roots, text: &str) -> String {
    let mut text = text.to_string();
    for root in &roots.roots {
        if let Some(path) = &root.physical {
            text = text
                .replace(&crate::local_access::display_path(path), &root.visible)
                .replace(
                    &crate::local_access::display_path(path).replace('\\', "/"),
                    &root.visible,
                );
        }
    }
    text
}

pub(in crate::share) fn current(progress: &ReclaimProgress, root: &str) -> String {
    let current = progress.stage.current_text();
    let current = if current.is_empty() { root } else { &current };
    let mut start = current.len().saturating_sub(4093);
    while !current.is_char_boundary(start) {
        start += 1;
    }
    if start == 0 {
        current.into()
    } else {
        format!("…{}", &current[start..])
    }
}

pub(in crate::share) fn send(
    report: DuplicateReport,
    progress: &Progress,
    mut emit: impl FnMut(FsDuplicateMessage) -> io::Result<()>,
) -> io::Result<()> {
    for group in report.groups {
        let wire = FsDuplicateGroup::of(&group, str::to_owned);
        let mut files = wire.files.into_iter().peekable();
        while files.peek().is_some() {
            progress.check_cancel()?;
            let mut portion = FsDuplicateGroup {
                sha256: wire.sha256.clone(),
                size: wire.size,
                files: Vec::new(),
                more: false,
            };
            let mut bytes = 128usize;
            while let Some(file) = files.peek() {
                let cost = file.path.len().saturating_mul(6) + 64;
                if !portion.files.is_empty()
                    && bytes + cost > crate::share::fs_response::GROUP_PORTION_BYTES
                {
                    break;
                }
                if cost > crate::share::fs_response::GROUP_PORTION_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Duplikatpfad überschreitet das Drahtformat",
                    ));
                }
                bytes += cost;
                if let Some(file) = files.next() {
                    portion.files.push(file);
                }
            }
            portion.more = files.peek().is_some();
            emit(FsDuplicateMessage::Groups {
                groups: vec![portion],
            })?;
        }
    }
    let mut summary = FsDuplicateSummary::of(report.summary, report.root_error);
    summary.fit_wire()?;
    emit(FsDuplicateMessage::Done { summary })
}
