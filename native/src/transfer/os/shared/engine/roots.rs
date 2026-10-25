//! Where the selected entries land (K14): relative targets as before (below
//! `base`, else by name, flattened by name), selected-folder aliases reserved
//! independently of scaffold parents, explicit pairs
//! validated as a whole, and the resolved roots of an earlier run reused for
//! "transfer missing files" (K8).
use super::super::engine_names::{
    base_name, first_component, root_rel, validate_pair_rels, with_first, NamePlanner,
};
use super::super::flow::{classify_error, Flow};
use super::super::flow_control::OpOutcome;
use super::super::job::{JobItems, Layout, PairItem};
use super::super::types::ResolvedRoot;
use super::super::walk::WalkRoot;
use super::super::walk_listers::native;
use super::folders::FolderRegister;
use super::queue::FileWork;
use super::view::{inside_source_message, JobView, Side};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A selected entry (for pairs: their top-level name) and its planned
/// destination relative to the target folder.
#[derive(Clone, Debug)]
pub(crate) struct PlannedRoot {
    pub source: String,
    pub planned: String,
}

#[derive(Default)]
pub(crate) struct RootPlan {
    /// Keep the cross-process job lock until discovery/apply has finished.
    pub _reservations: Option<super::folder_reservation::Reservations>,
    pub walk_roots: Vec<WalkRoot>,
    /// Explicit files of a `Pairs` job; `size_known` false ones are asked.
    pub pairs: Vec<(FileWork, bool)>,
    pub roots: Vec<PlannedRoot>,
    /// Pair destinations left out as protected omissions.
    pub omitted: u64,
}

/// The names below a remote target folder, from one listing (K14). A folder
/// that does not exist yet is created; a listing that fails for a transient
/// reason leaves the numbering to the exclusive creations.
pub(crate) fn remote_planner(
    view: &JobView<'_>,
    flow: &Arc<Flow>,
    cancel: &AtomicBool,
) -> Result<Option<NamePlanner>, String> {
    let Side::Remote(backend) = view.target else {
        return Ok(None);
    };
    let case_sensitive = backend.case_sensitive_paths(view.target_dir);
    let Some(permit) = flow.acquire_meta(cancel) else {
        return Ok(None);
    };
    let listed = backend.list_dir(view.target_dir);
    permit.finish(match &listed {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    match listed {
        Ok(entries) => Ok(Some(NamePlanner::new(
            entries.into_iter().map(|entry| entry.name),
            case_sensitive,
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            backend
                .mkdir_all(view.target_dir)
                .map_err(|error| format!("Zielordner „{}“ anlegen: {error}", view.target_dir))?;
            Ok(Some(NamePlanner::new(Vec::new(), case_sensitive)))
        }
        Err(error)
            if super::super::engine_policy::is_transient(
                error.kind(),
                classify_error(&error) == OpOutcome::Overload,
            ) =>
        {
            Ok(Some(NamePlanner::new(Vec::new(), case_sensitive)))
        }
        Err(error) => Err(format!("Zielordner „{}“ lesen: {error}", view.target_dir)),
    }
}

/// Plans the roots of `view` into `register` (which already knows the target
/// names when the target is remote).
pub(crate) fn plan(view: &JobView<'_>, register: &FolderRegister<'_>, cancel: &AtomicBool, completed: &[ResolvedRoot], files: &[ResolvedRoot], alias_required: bool) -> Result<RootPlan, String> {
    let resumed: HashMap<&str, &str> = view
        .resume
        .unwrap_or_default()
        .iter()
        .chain(completed)
        .chain(files)
        .map(|root| (root.source.as_str(), root.rel.as_str()))
        .collect();
    match view.items {
        JobItems::Roots { paths, base } => {
            plan_roots(view, register, paths, base.as_deref(), &resumed, cancel, completed, alias_required)
        }
        JobItems::Pairs(pairs) => plan_pairs(view, register, pairs, &resumed, completed),
    }
}

fn plan_roots(
    view: &JobView<'_>,
    register: &FolderRegister<'_>,
    paths: &[String],
    base: Option<&str>,
    resumed: &HashMap<&str, &str>,
    cancel: &AtomicBool,
    completed: &[ResolvedRoot],
    alias_required: bool,
) -> Result<RootPlan, String> {
    let allow_backslash = view.allow_backslash();
    let mut plan = RootPlan::default();
    for path in paths {
        let wanted = match view.layout {
            Layout::Tree => root_rel(path, base),
            Layout::Flatten => base_name(path).to_string(),
        };
        let invalid = wanted
            .split('/')
            .find_map(|component| name_problem(component, allow_backslash));
        if let Some(problem) = invalid {
            return Err(format!("{path}: {problem}"));
        }
        let rel = if let Some(rel) = resumed.get(path.as_str()) {
            (*rel).to_string()
        } else {
            wanted
        };
        plan.roots.push(PlannedRoot {
            source: path.clone(),
            planned: rel.clone(),
        });
        if completed.iter().any(|root| root.source == *path) { continue; }
        plan.walk_roots.push(WalkRoot {
            path: path.clone(),
            rel,
        });
    }
    // Canonical self-copy guards precede any selected-folder reservation.
    check_local_targets(view, &plan)?;
    if view.layout == Layout::Flatten {
        return Ok(plan);
    }
    let needs_aliases = view.resume.is_some() || plan.walk_roots.iter().any(|root| view.folders.for_root(&root.path) != crate::transfer::FolderConflict::Merge);
    let reservations = if needs_aliases {
        let namespace = |side: Side<'_>| side.backend().map_or_else(|| "local".to_string(), |backend| backend.namespace_identity());
        Some(super::folder_reservation::Reservations::open(super::super::super::folder_alias::FolderAliasScope {
            job: view.id.map(str::to_string).unwrap_or_else(super::new_job_id),
            source_namespace: namespace(view.source),
            target_namespace: namespace(view.target),
            target_root: register.root().to_string(),
            native_target: view.target.is_local(),
            vault_locations: super::record_io::record_locations(view.source, view.target, &paths.iter().map(String::as_str).collect::<Vec<_>>(), register.root()).map_err(|error| error.to_string())?,
        }, view.recovery, alias_required).map_err(|error| format!("Ordnerzuordnung speichern: {error}"))?)
    } else { None };
    let mut kept = Vec::new();
    let gate = view.source.is_local().then(|| crate::transfer::AccessGate::new(&view.gate_root()));
    for mut root in plan.walk_roots.drain(..) {
        if cancel.load(Ordering::Acquire) { return Err(super::super::cancel::CANCELED_ERROR.to_string()); }
        if view.resume.is_some() {
            if let Some(reservations) = &reservations {
                if !resumed.contains_key(root.path.as_str()) {
                    if let Some(relative) = reservations.saved_relative(&root.path) { root.rel = relative; }
                }
                let moved = reservations.confirm_resume(view.target, register.root(), &root.path, &root.rel).map_err(|error| format!("Wiederaufnahmealias bestätigen: {error}"))?;
                if let Some(entry) = plan.roots.iter_mut().find(|entry| entry.source == root.path) { entry.planned = root.rel.clone(); }
                if moved == Some(true) && !target_exists(view.source, &root.path)? { continue; }
            }
            kept.push(root);
            continue;
        }
        let saved_move = reservations.as_ref().is_some_and(|reservations| reservations.saved_move(&root.path));
        if saved_move || selected_directory(view.source, &root.path, gate.as_ref())? {
            let policy = view.folders.for_root(&root.path);
            if policy != crate::transfer::FolderConflict::Merge {
                if policy == crate::transfer::FolderConflict::Skip {
                    plan.omitted += 1;
                    plan.roots.retain(|entry| entry.source != root.path);
                    continue;
                }
                if let Some(parent) = super::super::engine_names::parent_rel(&root.rel) {
                    register.ensure(parent, cancel).map_err(|error| error.message)?;
                }
                if let Some(reservations) = &reservations {
                    let whole_move = view.mode == crate::types::CopyMode::Move && view.source.is_local() && view.target.is_local() && view.filter.is_none();
                    if !whole_move {
                    match reservations.reserve(view.target, register.root(), &root.path, &root.rel, policy == crate::transfer::FolderConflict::KeepBoth, cancel) {
                        Ok(relative) => root.rel = relative,
                        Err(error) if policy == crate::transfer::FolderConflict::Skip && error.kind() == std::io::ErrorKind::AlreadyExists => {
                            plan.omitted += 1;
                            plan.roots.retain(|entry| entry.source != root.path);
                            continue;
                        }
                        Err(error) => return Err(format!("Zielordner für „{}“ reservieren: {error}", root.path)),
                    }
                    }
                }
            }
            if let Some(entry) = plan.roots.iter_mut().find(|entry| entry.source == root.path) {
                entry.planned = root.rel.clone();
            }
        } else if !view.target.is_local() && !root.rel.contains('/') && !resumed.contains_key(root.path.as_str()) {
            root.rel = claim(register, &root.rel)?;
            if let Some(entry) = plan.roots.iter_mut().find(|entry| entry.source == root.path) { entry.planned = root.rel.clone(); }
        }
        kept.push(root);
    }
    plan.walk_roots = kept;
    plan._reservations = reservations;
    Ok(plan)
}

fn selected_directory(side: Side<'_>, path: &str, gate: Option<&crate::transfer::AccessGate>) -> Result<bool, String> {
    let inspect = || match side {
        Side::Local => crate::local_access::symlink_metadata(&native(path))
            .map(|meta| meta.is_dir() && !crate::local_access::metadata_is_link_like(&native(path), &meta)),
        Side::Remote(backend) => backend.stat(path).map(|meta| meta.is_dir && !meta.is_symlink),
    };
    let mut result = inspect();
    if result.as_ref().is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied) {
        if let Some(gate) = gate {
            if gate.request() == crate::transfer::AccessAnswer::Granted { result = inspect(); }
        }
    }
    result.map_err(|error| format!("Quellordner „{path}“ prüfen: {error}"))
}

fn target_exists(side: Side<'_>, path: &str) -> Result<bool, String> {
    let result = match side {
        Side::Local => crate::local_access::symlink_metadata(&native(path)).map(|_| ()),
        Side::Remote(backend) => backend.stat(path).map(|_| ()),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Zielordner „{path}“ prüfen: {error}")),
    }
}

fn plan_pairs(
    view: &JobView<'_>,
    register: &FolderRegister<'_>,
    pairs: &[PairItem],
    resumed: &HashMap<&str, &str>,
    completed: &[ResolvedRoot],
) -> Result<RootPlan, String> {
    let allow_backslash = view.allow_backslash();
    validate_pair_rels(pairs.iter().map(|pair| pair.rel.as_str()), allow_backslash)?;
    let mut plan = RootPlan::default();
    let mut firsts: HashMap<String, String> = HashMap::new();
    for pair in pairs {
        let receipt_key = format!("{}\0{}", pair.source, pair.rel);
        if pair.rel.split('/').any(crate::apptrash::excluded_name) {
            plan.omitted += 1;
            continue;
        }
        let first = first_component(&pair.rel).to_string();
        if let Some(root) = completed.iter().find(|root| root.source == receipt_key) {
            if !firsts.contains_key(&first) {
                let planned = first_component(&root.rel).to_string();
                plan.roots.push(PlannedRoot { source: first.clone(), planned: planned.clone() });
                firsts.insert(first, planned);
            }
            continue;
        }
        let planned = match firsts.get(&first) {
            Some(planned) => planned.clone(),
            None => {
                let planned = match resumed.get(receipt_key.as_str()).or_else(|| resumed.get(first.as_str())) {
                    Some(rel) => first_component(rel).to_string(),
                    None if view.target.is_local() || pair.rel.contains('/') => first.clone(),
                    None => claim(register, &first)?,
                };
                plan.roots.push(PlannedRoot {
                    source: first.clone(),
                    planned: planned.clone(),
                });
                firsts.insert(first, planned.clone());
                planned
            }
        };
        let mut work = FileWork::new(
            pair.source.clone(),
            with_first(&pair.rel, &planned),
            pair.size.unwrap_or(0),
            pair.mtime_ms,
        );
        work.id = pair.id.clone();
        work.receipt_key = Some(receipt_key);
        plan.pairs.push((work, pair.size.is_some()));
    }
    Ok(plan)
}

fn claim(register: &FolderRegister<'_>, wanted: &str) -> Result<String, String> {
    register
        .claim_name(wanted)
        .ok_or_else(|| format!("Kein freier Name für „{wanted}“ im Zielordner"))
}

fn name_problem(component: &str, allow_backslash: bool) -> Option<String> {
    super::super::engine_names::name_problem(component, allow_backslash)
}

/// Local copies: no selected folder may receive itself, compared on
/// canonical paths (links and case variants included, K1).
pub(crate) fn check_local_targets(view: &JobView<'_>, plan: &RootPlan) -> Result<(), String> {
    if !(view.source.is_local() && view.target.is_local()) {
        return Ok(());
    }
    for root in &plan.walk_roots {
        let source = native(&root.path);
        let is_dir = crate::local_access::symlink_metadata(&source)
            .map(|metadata| {
                metadata.is_dir() && !crate::local_access::metadata_is_link_like(&source, &metadata)
            })
            .unwrap_or(false);
        if !is_dir {
            continue;
        }
        let destination = match view.layout {
            Layout::Tree => native(&super::super::engine_names::join_rel(
                view.target_dir,
                &root.rel,
            )),
            Layout::Flatten => native(view.target_dir),
        };
        if crate::copy::validate_directory_target(&source, &destination).is_err()
            || crate::copy::validate_directory_target(&source, &native(view.target_dir)).is_err()
        {
            return Err(inside_source_message(&root.path));
        }
    }
    Ok(())
}

/// The roots a finished run resolved, for "transfer missing files".
pub(crate) fn resolved(plan: &RootPlan, register: &FolderRegister<'_>) -> Vec<ResolvedRoot> {
    plan.roots
        .iter()
        .map(|root| ResolvedRoot {
            source: root.source.clone(),
            rel: register.actual_rel(&root.planned),
        })
        .collect()
}
