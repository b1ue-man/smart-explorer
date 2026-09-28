//! Route one authorized export to the exact local analytics worker.
use crate::analytics::{Progress, ScanOutcome, ScanPhase, ScanStatus, SizeNode};
use crate::share::{fs, fs_access::FsAccess};
use crate::vfs::Scheme;
use std::{io, path::Path, sync::atomic::Ordering};

pub(in crate::share) fn scan(root: &str, access: &FsAccess, progress: &Progress) -> ScanOutcome {
    match scan_inner(root, access, progress) {
        Ok(outcome) => outcome,
        Err(_) if progress.check_cancel().is_err() => ScanOutcome::canceled(),
        Err(error) => {
            let denied = error.kind() == io::ErrorKind::PermissionDenied;
            let mut outcome = ScanOutcome::failed(root, error.to_string());
            outcome.permission_denied = u64::from(denied);
            outcome
        }
    }
}

fn scan_inner(root: &str, access: &FsAccess, progress: &Progress) -> io::Result<ScanOutcome> {
    progress.check_cancel()?;
    let parts = fs::split_clean(root)?;
    let root = if parts.is_empty() { "/".into() } else { format!("/{}", parts.join("/")) };
    progress.set_phase(ScanPhase::Preparing, &root);
    if matches!(access, FsAccess::Dynamic(_))
        && (parts.is_empty() || (parts.len() == 1 && parts[0] == "Verbindungen"))
    {
        return scan_container(&root, access, progress);
    }
    // Retain the connection/backend for the entire subtree. A remote locator
    // never passes through Path or any local filesystem call.
    let target = access.resolve(&root)?;
    let mut outcome = if target.backend.scheme() == Scheme::Local {
        scan_local_target(&target.path, &root, progress)?
    } else {
        let scoped = progress.scoped(target.path.clone(), root.clone());
        let mut outcome = crate::analytics::scan_remote(&*target.backend, &target.path, &scoped);
        remap_issues(&mut outcome, &scoped, &target.path);
        outcome
    };
    if let Some(tree) = &mut outcome.tree {
        tree.name = parts.last().map_or("/", String::as_str).into();
    }
    Ok(outcome)
}

fn scan_local_target(physical: &str, visible: &str, progress: &Progress) -> io::Result<ScanOutcome> {
    let root = crate::local_access::normalize_scan_root(Path::new(physical));
    let canonical = std::fs::canonicalize(&root)?;
    let scoped = progress.scoped(
        crate::local_access::display_path(&root).replace('\\', "/"), visible.into(),
    );
    let guard = |path: &Path| {
        let metadata = crate::local_access::symlink_metadata(path)?;
        if !metadata.is_dir() || crate::local_access::metadata_is_link_like(path, &metadata) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "Analyse-Ordner wurde durch einen Link oder eine Datei ersetzt"));
        }
        if !std::fs::canonicalize(path)?.starts_with(&canonical) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "Analyse-Pfad liegt außerhalb der freigegebenen Wurzel"));
        }
        Ok(())
    };
    // Same worker, enumeration, pool size, authority check, retention and error
    // handling as the GUI's local drive scan. Only Share confinement is added.
    let mut outcome = crate::analytics::scan_with_guard(&root, &scoped, Some(&guard));
    remap_issues(&mut outcome, &scoped, physical);
    Ok(outcome)
}

fn remap_issues(outcome: &mut ScanOutcome, scoped: &Progress, physical: &str) {
    let visible = scoped.visible_path(physical);
    for issue in &mut outcome.issues {
        issue.path = scoped.visible_path(&issue.path);
        issue.detail = issue.detail.replace(physical, &visible)
            .replace(&physical.replace('/', "\\"), &visible);
    }
    for note in &mut outcome.notes {
        *note = note.replace(physical, &visible).replace(&physical.replace('/', "\\"), &visible);
    }
}

fn scan_container(root: &str, access: &FsAccess, progress: &Progress) -> io::Result<ScanOutcome> {
    let entries = access.list_dir(root)?;
    let mut result = ScanOutcome::complete(SizeNode {
        name: root.rsplit('/').find(|name| !name.is_empty()).unwrap_or("/").into(),
        size: 0, is_dir: true, children: Vec::new(),
    });
    for entry in entries {
        progress.check_cancel()?;
        crate::vfs::validate_child_name(&entry.name)?;
        if entry.is_symlink { continue; }
        if !entry.is_dir { return Err(io::Error::other("Freigabe-Container enthält eine Datei")); }
        progress.dirs.fetch_add(1, Ordering::Relaxed);
        let path = format!("{}/{}", root.trim_end_matches('/'), entry.name);
        let mut child = scan(&path, access, progress);
        if child.status == ScanStatus::Canceled { return Ok(child); }
        if child.status != ScanStatus::Complete { result.status = ScanStatus::Partial; }
        result.permission_denied += child.permission_denied;
        result.suppressed_issues += child.suppressed_issues;
        result.aggregated_files += child.aggregated_files;
        for issue in child.issues {
            if result.issues.len() < 64 { result.issues.push(issue); }
            else { result.suppressed_issues += 1; }
        }
        for note in child.notes {
            if result.notes.len() < 16 { result.notes.push(note); }
        }
        let node = child.tree.take().unwrap_or(SizeNode {
            name: entry.name.into(), size: 0, is_dir: true, children: Vec::new(),
        });
        if let Some(tree) = &mut result.tree {
            tree.size = tree.size.checked_add(node.size).ok_or_else(|| io::Error::other("Größenüberlauf"))?;
            tree.children.push(node);
        }
    }
    Ok(result)
}
