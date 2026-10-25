//! `fs.transfer`: one engine job for local and connected endpoints. Folder
//! decisions are independent of the local file-conflict choice. Discovery
//! streams; filter/baseDir retain their literal relative paths.
use super::args::{filter_arg, nonempty_list, opt_str, str_arg};
use super::crumbs;
use super::drive::{run_job, Outcome};
use super::error::ApiError;
use super::fs_list::{reject_trash, require_writable};
use super::location::{is_same_or_below, parent_path, Loc};
use super::runtime::{Runtime, TaskCtx};
use crate::filter::CompiledFilter;
use crate::transfer::{Endpoint, FolderConflict, FolderConflictPolicy, JobItems, Layout, TransferJob};
use crate::types::{Conflict, CopyMode, FilterDef};
use serde_json::{json, Value};
#[cfg(test)]
use std::sync::Arc;

struct Plan {
    sources: Vec<Loc>,
    target: Loc,
    mode: CopyMode,
    conflict: Conflict,
    folders: FolderConflictPolicy,
    /// The filter and the backend path the relative paths start at.
    filter: Option<(FilterDef, String)>,
}

pub(crate) fn transfer(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let plan = parse(args)?;
    let verb = match plan.mode {
        CopyMode::Copy => "Kopieren",
        CopyMode::Move => "Verschieben",
    };
    let title = format!(
        "{verb}: {} Element(e) → {}",
        plan.sources.len(),
        crumbs::task_title(&plan.target, &rt.volumes(), &super::domains::vault_label)
    );
    let private = plan.target.is_vault() || plan.sources.iter().any(Loc::is_vault);
    let runtime = rt.clone();
    let id = rt.spawn_task("transfer", title, move |ctx| {
        if private {
            ctx.keep_names_private();
        }
        run(&runtime, ctx, &plan)?.into_result(ctx)
    });
    Ok(json!({ "taskId": id }))
}

fn parse(args: &Value) -> Result<Plan, ApiError> {
    let sources = nonempty_list(args, "sources")?
        .iter()
        .map(|source| Loc::parse(source))
        .collect::<Result<Vec<_>, _>>()?;
    let first = &sources[0];
    for source in &sources {
        reject_trash(source)?;
        if source.is_root() && !source.is_local() {
            return Err(ApiError::invalid(
                "Die Wurzel einer Verbindung kann nicht kopiert werden",
            ));
        }
        if source.kind != first.kind || source.prefix != first.prefix {
            return Err(ApiError::invalid(
                "Alle Quellen müssen am selben Ort liegen",
            ));
        }
    }
    let target = Loc::parse(str_arg(args, "targetDir")?)?;
    require_writable(&target)?;
    let mode = match opt_str(args, "mode").unwrap_or("copy") {
        "copy" => CopyMode::Copy,
        "move" => CopyMode::Move,
        other => return Err(ApiError::invalid(format!("Unbekannter Modus: {other}"))),
    };
    if mode == CopyMode::Move && !(first.is_local() && target.is_local()) {
        return Err(ApiError::unsupported(
            "Verschieben von/zu Remote wird nicht unterstützt – bitte kopieren",
        ));
    }
    let conflict = match opt_str(args, "conflict").unwrap_or("keepBoth") {
        "skip" => Conflict::Skip,
        "replace" => Conflict::Overwrite,
        "keepBoth" => Conflict::Rename,
        other => {
            return Err(ApiError::invalid(format!(
                "Unbekannte Konfliktregel: {other}"
            )))
        }
    };
    let same_place = target.kind == first.kind && target.prefix == first.prefix;
    let parse_folder = |value: &str| match value {
        "merge" => Ok(FolderConflict::Merge),
        "keep-both" | "keepBoth" => Ok(FolderConflict::KeepBoth),
        "skip" => Ok(FolderConflict::Skip),
        "fail" => Ok(FolderConflict::Fail),
        value => Err(ApiError::invalid(format!("Unbekannte Ordnerkonfliktregel: {value}"))),
    };
    let mut folders = FolderConflictPolicy::new(parse_folder(opt_str(args, "folderConflict").unwrap_or("keep-both"))?);
    if let Some(choices) = args.get("folderConflicts").and_then(Value::as_object) {
        for (location, choice) in choices {
            let source = Loc::parse(location)?;
            if !sources.iter().any(|item| item.kind == source.kind && item.prefix == source.prefix && item.path == source.path) {
                return Err(ApiError::invalid("Ordnerentscheidung gehört nicht zur Quellauswahl"));
            }
            let choice = choice.as_str().ok_or_else(|| ApiError::invalid("Ordnerentscheidung muss ein Text sein"))?;
            folders.roots.insert(source.path, parse_folder(choice)?);
        }
    }
    if same_place
        && sources
            .iter()
            .any(|source| is_same_or_below(&target.path, &source.path))
    {
        return Err(ApiError::invalid("Das Ziel liegt in einer der Quellen"));
    }
    // `Filter.hidden` decides (Kotlin sets it when the scan view showed hidden entries).
    let filter = match (filter_arg(args, "filter", false)?, opt_str(args, "baseDir")) {
        (Some(filter), Some(base)) => {
            let base = Loc::parse(base)?;
            if base.kind != first.kind || base.prefix != first.prefix {
                return Err(ApiError::invalid(
                    "„baseDir“ liegt nicht am Ort der Quellen",
                ));
            }
            if let Some(error) = CompiledFilter::compile(&filter).error() {
                return Err(ApiError::invalid(error.to_string()));
            }
            if sources.iter().any(|source| {
                !is_same_or_below(&source.path, &base.path) || source.path == base.path
            }) {
                return Err(ApiError::invalid(
                    "Die Quellen liegen nicht unter „baseDir“",
                ));
            }
            Some((filter, base.path))
        }
        _ => None,
    };
    Ok(Plan {
        sources,
        target,
        mode,
        conflict,
        folders,
        filter,
    })
}

fn run(rt: &Runtime, ctx: &TaskCtx, plan: &Plan) -> Result<Outcome, ApiError> {
    let paths: Vec<String> = plan
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect();
    let source = endpoint(rt, &plan.sources[0])?;
    let target = endpoint(rt, &plan.target)?;
    let volumes = rt.volumes();
    let label = &super::domains::vault_label;
    let labels = (
        crumbs::task_title(&plan.sources[0], &volumes, label),
        crumbs::task_title(&plan.target, &volumes, label),
    );
    run_job(ctx, plan_job(plan, paths, source, target, labels))
}

/// The local filesystem or the live connection of `loc`.
fn endpoint(rt: &Runtime, loc: &Loc) -> Result<Endpoint, ApiError> {
    if loc.is_local() {
        Ok(Endpoint::Local)
    } else {
        Ok(Endpoint::Remote(rt.resolve_live(loc)?.0))
    }
}

pub(crate) fn preview_job(rt: &Runtime, args: &Value) -> Result<TransferJob, ApiError> {
    let plan = parse(args)?;
    let source = endpoint(rt, &plan.sources[0])?;
    let target = endpoint(rt, &plan.target)?;
    let paths = plan.sources.iter().map(|source| source.path.clone()).collect();
    Ok(plan_job(&plan, paths, source, target, (String::new(), String::new())))
}

/// The engine job of a transfer that involves a connection: the selected
/// entries under their names, or with `filter` + `baseDir` the matching
/// files below them at their paths relative to `baseDir` (found while
/// copying). `conflict` applies to local→local only (api.md): here occupied
/// names are numbered and nothing is replaced.
fn plan_job(
    plan: &Plan,
    paths: Vec<String>,
    source: Endpoint,
    target: Endpoint,
    (source_label, target_label): (String, String),
) -> TransferJob {
    TransferJob {
        id: crate::transfer::new_job_id(),
        recoverable: true,
        recovery: crate::transfer::RecoveryMode::New,
        folders: plan.folders.clone(),
        source,
        target,
        target_dir: plan.target.path.clone(),
        items: JobItems::Roots {
            // Destinations are cut below the base without its trailing slash
            // (a volume root `/` becomes the empty base).
            base: plan.filter.as_ref().map(|(_, base)| base.trim_end_matches('/').to_string())
                .or_else(|| (plan.sources[0].is_local() && plan.target.is_local()).then(|| common_parent(&paths).trim_end_matches('/').to_string())),
            paths,
        },
        layout: Layout::Tree,
        filter: plan.filter.clone(),
        conflict: if plan.sources[0].is_local() && plan.target.is_local() { plan.conflict } else { Conflict::Rename },
        mode: plan.mode,
        source_label,
        target_label,
        resume: None,
    }
}

/// The deepest folder that contains every path.
pub(crate) fn common_parent(paths: &[String]) -> String {
    let mut common: Vec<&str> = match paths.first().and_then(|path| parent_path(path)) {
        Some(parent) => parent.split('/').filter(|part| !part.is_empty()).collect(),
        None => return "/".to_string(),
    };
    for path in paths.iter().skip(1) {
        let parent: Vec<&str> = parent_path(path)
            .unwrap_or("/")
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        let shared = common
            .iter()
            .zip(&parent)
            .take_while(|(left, right)| left == right)
            .count();
        common.truncate(shared);
    }
    format!("/{}", common.join("/"))
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
