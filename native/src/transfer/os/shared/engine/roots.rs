//! Where the selected entries land (K14): relative targets as before (below
//! `base`, else by name, flattened by name), new top-level names at remote
//! targets chosen from one listing of the target folder, explicit pairs
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
use std::sync::atomic::AtomicBool;
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
pub(crate) fn plan(view: &JobView<'_>, register: &FolderRegister<'_>) -> Result<RootPlan, String> {
    let resumed: HashMap<&str, &str> = view
        .resume
        .unwrap_or_default()
        .iter()
        .map(|root| (root.source.as_str(), root.rel.as_str()))
        .collect();
    match view.items {
        JobItems::Roots { paths, base } => {
            plan_roots(view, register, paths, base.as_deref(), &resumed)
        }
        JobItems::Pairs(pairs) => plan_pairs(view, register, pairs, &resumed),
    }
}

fn plan_roots(
    view: &JobView<'_>,
    register: &FolderRegister<'_>,
    paths: &[String],
    base: Option<&str>,
    resumed: &HashMap<&str, &str>,
) -> Result<RootPlan, String> {
    let allow_backslash = view.allow_backslash();
    let mut plan = RootPlan::default();
    let mut shared: HashMap<String, String> = HashMap::new();
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
        } else if view.layout == Layout::Flatten || view.target.is_local() {
            wanted.clone()
        } else if wanted.contains('/') {
            let first = first_component(&wanted).to_string();
            let planned = match shared.get(&first) {
                Some(planned) => planned.clone(),
                None => {
                    let planned = claim(register, &first)?;
                    register.plan_exclusive(&planned, &first);
                    shared.insert(first, planned.clone());
                    planned
                }
            };
            with_first(&wanted, &planned)
        } else {
            let planned = claim(register, &wanted)?;
            register.plan_exclusive(&planned, &wanted);
            planned
        };
        plan.roots.push(PlannedRoot {
            source: path.clone(),
            planned: rel.clone(),
        });
        plan.walk_roots.push(WalkRoot {
            path: path.clone(),
            rel,
        });
    }
    Ok(plan)
}

fn plan_pairs(
    view: &JobView<'_>,
    register: &FolderRegister<'_>,
    pairs: &[PairItem],
    resumed: &HashMap<&str, &str>,
) -> Result<RootPlan, String> {
    let allow_backslash = view.allow_backslash();
    validate_pair_rels(pairs.iter().map(|pair| pair.rel.as_str()), allow_backslash)?;
    let mut plan = RootPlan::default();
    let mut firsts: HashMap<String, String> = HashMap::new();
    for pair in pairs {
        if pair.rel.split('/').any(crate::apptrash::excluded_name) {
            plan.omitted += 1;
            continue;
        }
        let first = first_component(&pair.rel).to_string();
        let planned = match firsts.get(&first) {
            Some(planned) => planned.clone(),
            None => {
                let planned = match resumed.get(first.as_str()) {
                    Some(rel) => first_component(rel).to_string(),
                    None if view.target.is_local() => first.clone(),
                    None => {
                        let planned = claim(register, &first)?;
                        if pair.rel.contains('/') {
                            register.plan_exclusive(&planned, &first);
                        }
                        planned
                    }
                };
                plan.roots.push(PlannedRoot {
                    source: first.clone(),
                    planned: planned.clone(),
                });
                firsts.insert(first, planned.clone());
                planned
            }
        };
        let work = FileWork {
            source: pair.source.clone(),
            rel: with_first(&pair.rel, &planned),
            size: pair.size.unwrap_or(0),
            mtime_ms: pair.mtime_ms,
            id: pair.id.clone(),
            md5: None,
            retried: false,
        };
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
