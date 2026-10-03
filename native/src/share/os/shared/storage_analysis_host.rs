//! Host analysis uses one budget and disjoint, handle-confined local roots.
use crate::analytics::{Progress, ScanBudget, ScanOutcome, ScanPhase, ScanStatus, SizeNode};
use crate::share::{fs_access::FsAccess, storage_roots::{self, Plan, Roots}};
use std::{io, sync::atomic::Ordering};

const MAX_NOTES: usize = 16;

pub(in crate::share) fn scan(root: &str, access: &FsAccess, p: &Progress) -> ScanOutcome {
    scan_bounded(root, access, p, None)
}

pub(in crate::share) fn scan_bounded(root: &str, access: &FsAccess, p: &Progress, nodes: Option<u64>) -> ScanOutcome {
    if let Some(nodes) = nodes { p.set_node_budget(nodes); }
    match scan_inner(root, access, p) {
        Ok(result) => result,
        Err(_) if p.check_cancel().is_err() => ScanOutcome::canceled(),
        Err(error) => {
            let mut result = ScanOutcome::failed(root, error.to_string());
            result.permission_denied = u64::from(error.kind() == io::ErrorKind::PermissionDenied);
            result
        }
    }
}

fn scan_inner(root: &str, access: &FsAccess, p: &Progress) -> io::Result<ScanOutcome> {
    p.check_cancel()?;
    p.set_phase(ScanPhase::Preparing, root);
    let roots = storage_roots::resolve(root, access, p)?;
    let budget = ScanBudget::with_node_limit(Some(p.node_budget()));
    let excluded = storage_roots::excluded();
    let mut result = visit(&roots.plan, &roots, p, &budget, &excluded)?;
    if result.status == ScanStatus::Canceled { return Ok(result); }
    if let Some(tree) = &mut result.tree {
        // Container and aggregate nodes consume the receiver's budget too.
        crate::analytics::fit_tree(tree, p.node_budget(), p)?;
        p.bytes.store(tree.size, Ordering::Relaxed);
    }
    protected_as_note(&mut result);
    if let Some(note) = crate::analytics::host_permission_note(result.permission_denied) {
        result.notes.push(note);
    }
    result.notes.truncate(MAX_NOTES);
    Ok(result)
}

fn visit(plan: &Plan, roots: &Roots, p: &Progress, budget: &ScanBudget, excluded: &[std::path::PathBuf]) -> io::Result<ScanOutcome> {
    p.check_cancel()?;
    match plan {
        Plan::Failed { visible, kind, detail } => {
            let mut result = ScanOutcome::failed(visible, detail);
            result.permission_denied = u64::from(*kind == io::ErrorKind::PermissionDenied);
            Ok(result)
        }
        Plan::Branch(visible, children) => {
            let mut result = ScanOutcome::complete(empty(visible));
            for child in children {
                p.dirs.fetch_add(1, Ordering::Relaxed);
                let mut outcome = visit(child, roots, p, budget, excluded)?;
                if outcome.status == ScanStatus::Canceled { return Ok(outcome); }
                if outcome.status != ScanStatus::Complete { result.status = ScanStatus::Partial; }
                result.permission_denied = result.permission_denied.saturating_add(outcome.permission_denied);
                result.suppressed_issues = result.suppressed_issues.saturating_add(outcome.suppressed_issues);
                result.aggregated_files = result.aggregated_files.saturating_add(outcome.aggregated_files);
                for issue in outcome.issues {
                    if result.issues.len() < 64 { result.issues.push(issue); }
                    else { result.suppressed_issues = result.suppressed_issues.saturating_add(1); }
                }
                result.protected.extend(outcome.protected);
                result.notes.extend(outcome.notes.into_iter().take(MAX_NOTES.saturating_sub(result.notes.len())));
                if let Some(node) = outcome.tree.take() {
                    if let Some(tree) = &mut result.tree {
                        tree.size = tree.size.checked_add(node.size).ok_or_else(|| io::Error::other("Größenüberlauf"))?;
                        tree.children.push(node);
                    }
                }
            }
            coalesce_protected(&mut result.protected);
            Ok(result)
        }
        Plan::Leaf(index) => {
            let root = &roots.roots[*index];
            if !root.retained {
                let mut result = ScanOutcome::complete(empty(&root.visible));
                result.notes.push(format!("{}: bereits durch eine andere Freigabewurzel erfasst", root.visible));
                return Ok(result);
            }
            let mut outcome = if let Some(physical) = &root.physical {
                let scoped = p.scoped(crate::local_access::display_path(physical), root.visible.clone());
                let mut result = match storage_roots::open_local(&root.target) {
                    Ok(handle) => crate::analytics::scan_confined_with(physical, &scoped, budget, excluded, handle),
                    Err(error) => {
                        let mut result = ScanOutcome::failed(&root.visible, error.to_string());
                        result.permission_denied = u64::from(error.kind() == io::ErrorKind::PermissionDenied);
                        result
                    }
                };
                remap(&mut result, &scoped, &crate::local_access::display_path(physical));
                result.volume = crate::analytics::host_volume_usage(physical).ok();
                if let Some((volume, totals, measured)) = crate::analytics::remembered_platform_totals() {
                    if let Ok(relative) = physical.strip_prefix(&volume) {
                        let segments: Vec<String> = relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
                        let place = crate::analytics::VolumeRoot::from_segments(&segments, true);
                        let mut figures = crate::analytics::PlatformFigures::new(&place, &totals, measured);
                        if !place.whole_volume { figures.apps.clear(); }
                        result.platform = Some(figures);
                    }
                }
                result
            } else {
                let scoped = p.scoped(root.target.path.clone(), root.visible.clone()).remote_segment();
                let mut result = crate::analytics::scan_remote(&*root.target.backend, &root.target.path, &scoped);
                remap(&mut result, &scoped, &root.target.path);
                result
            };
            if let Some(tree) = &mut outcome.tree { tree.name = segment(&root.visible).into(); }
            Ok(outcome)
        }
    }
}

fn segment(path: &str) -> &str { path.rsplit('/').find(|name| !name.is_empty()).unwrap_or("/") }
fn empty(path: &str) -> SizeNode {
    SizeNode { name: segment(path).into(), size: 0, is_dir: true, children: Vec::new() }
}
fn coalesce_protected(areas: &mut Vec<crate::analytics::ProtectedOmission>) {
    areas.sort_by(|a,b| a.area.cmp(&b.area));
    areas.dedup_by(|a,b| { if a.area == b.area { b.entries = b.entries.saturating_add(a.entries); true } else { false } });
}
fn protected_as_note(outcome: &mut ScanOutcome) {
    if let Some(note) = crate::analytics::protected_note(&outcome.protected) {
        outcome.notes.insert(0, note); outcome.notes.truncate(MAX_NOTES);
    }
}
fn remap(outcome: &mut ScanOutcome, scoped: &Progress, physical: &str) {
    let visible = scoped.visible_path(physical);
    let replace = |text: &str| text.replace(physical, &visible).replace(&physical.replace('/', "\\"), &visible);
    for issue in &mut outcome.issues { issue.path = scoped.visible_path(&issue.path); issue.detail = replace(&issue.detail); }
    for note in &mut outcome.notes { *note = replace(note); }
    for area in &mut outcome.protected { area.area = scoped.visible_path(&area.area); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_share_protected_areas_keep_structured_and_legacy_forms() {
        let mut result = ScanOutcome::protected_root("/", "/phone/Android/data");
        protected_as_note(&mut result);
        assert_eq!(result.status, ScanStatus::Complete);
        assert_eq!(result.protected.len(), 1);
        assert_eq!(result.notes.len(), 1);
        assert!(result.issues.is_empty());
    }
}
