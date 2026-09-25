//! `fs.transfer` (api.md §4.3): local→local through the desktop copy engine
//! (move and the conflict choice only there), everything involving a remote
//! through the desktop transfer workers (numbered names, never replacing).
//! With `filter` + `baseDir` only matching files go, keeping their paths
//! relative to `baseDir`.
use super::args::{filter_arg, nonempty_list, opt_str, str_arg};
use super::crumbs;
use super::drive::{drain_copy, run_transfer, Outcome};
use super::entry::extension;
use super::error::ApiError;
use super::fs_list::{reject_trash, require_writable};
use super::location::{is_same_or_below, parent_path, Loc};
use super::runtime::{Runtime, TaskCtx};
use crate::filter::CompiledFilter;
use crate::transfer::TransferRequest;
use crate::types::{Conflict, CopyMode, CopyOptions, FileEntry, FilterDef};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Plan {
    sources: Vec<Loc>,
    target: Loc,
    mode: CopyMode,
    conflict: Conflict,
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
        crumbs::title(&plan.target, &rt.volumes())
    );
    let runtime = rt.clone();
    let id = rt.spawn_transfer_task("transfer", title, move |ctx| {
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
        filter,
    })
}

fn run(rt: &Runtime, ctx: &TaskCtx, plan: &Plan) -> Result<Outcome, ApiError> {
    let paths: Vec<String> = plan
        .sources
        .iter()
        .map(|source| source.path.clone())
        .collect();
    let source_local = plan.sources[0].is_local();
    let dest_root = plan.target.path.clone();
    let request = match (source_local, plan.target.is_local()) {
        (true, true) => return local_copy(ctx, plan, paths),
        (true, false) => {
            let (backend, _) = rt.resolve_loc(&plan.target)?;
            match &plan.filter {
                Some((filter, base)) => TransferRequest::UploadPairs {
                    pairs: local_snapshot(ctx, &plan.sources, filter, base)?,
                    backend,
                    dest_root,
                },
                None => TransferRequest::Upload {
                    paths,
                    backend,
                    dest_root,
                },
            }
        }
        (false, true) => TransferRequest::Download {
            backend: rt.resolve_loc(&plan.sources[0])?.0,
            files: paths,
            dest_local: dest_root,
            filter: plan.filter.clone(),
        },
        (false, false) => TransferRequest::RemoteCopy {
            src: rt.resolve_loc(&plan.sources[0])?.0,
            files: paths,
            tgt: rt.resolve_loc(&plan.target)?.0,
            dest_root,
            filter: plan.filter.clone(),
        },
    };
    run_transfer(ctx, request)
}

fn local_copy(ctx: &TaskCtx, plan: &Plan, paths: Vec<String>) -> Result<Outcome, ApiError> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let dest = PathBuf::from(&plan.target.path);
    let handle = match &plan.filter {
        Some((filter, base)) => crate::copy::start_copy_expanded(
            seeds(&plan.sources, base)?,
            Some((filter.clone(), base.clone())),
            CopyOptions {
                root: PathBuf::from(base),
                dest,
                preserve_structure: true,
                conflict: plan.conflict,
                mode: plan.mode,
            },
            tx,
        ),
        None => {
            let root = common_parent(&paths);
            crate::copy::start_copy_from_paths(
                paths,
                CopyOptions {
                    root: PathBuf::from(root),
                    dest,
                    preserve_structure: true,
                    conflict: plan.conflict,
                    mode: plan.mode,
                },
                tx,
            )
        }
    };
    let mut outcome = drain_copy(ctx, handle, rx);
    outcome.omitted = trash_roots(&plan.sources);
    Ok(outcome)
}

/// Sources that contain the active app trash (volume roots); the copy engine
/// leaves it out.
fn trash_roots(sources: &[Loc]) -> u64 {
    let name = crate::apptrash::TRASH_DIR_NAME;
    if !crate::apptrash::excluded_name(name) {
        return 0;
    }
    sources
        .iter()
        .filter(|source| Path::new(&source.path).join(name).is_dir())
        .count() as u64
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

/// Selected local rows as copy seeds, with depths relative to `base`.
fn seeds(sources: &[Loc], base: &str) -> Result<Vec<FileEntry>, ApiError> {
    sources
        .iter()
        .map(|source| {
            let metadata = std::fs::symlink_metadata(&source.path)
                .map_err(|error| ApiError::from(error).context(&source.path))?;
            let name = source.name().to_string();
            let is_dir = metadata.is_dir();
            let depth = source
                .path
                .strip_prefix(base.trim_end_matches('/'))
                .unwrap_or(&source.path)
                .split('/')
                .filter(|part| !part.is_empty())
                .count() as u32;
            Ok(FileEntry {
                path: Arc::from(source.path.as_str()),
                parent: Arc::from(parent_path(&source.path).unwrap_or("/")),
                name: Arc::from(name.as_str()),
                ext: Arc::from(extension(&name, is_dir).as_str()),
                size: if is_dir { 0 } else { metadata.len() },
                mtime_ms: modified_ms(&metadata),
                btime_ms: 0,
                is_dir,
                is_symlink: metadata.file_type().is_symlink(),
                hidden: name.starts_with('.'),
                system: false,
                depth,
                id: None,
            })
        })
        .collect()
}

fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis() as i64)
}

/// Matching local files below the selection as `(absolute, relative)` pairs.
fn local_snapshot(
    ctx: &TaskCtx,
    sources: &[Loc],
    filter: &FilterDef,
    base: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    let compiled = CompiledFilter::compile(filter);
    let cancel = ctx.cancel_flag();
    let mut files = Vec::new();
    for seed in seeds(sources, base)? {
        if !seed.is_dir {
            files.push(seed);
            continue;
        }
        if seed.is_symlink {
            continue;
        }
        let outcome = crate::scanner::collect_recursive(
            Path::new(seed.path.as_ref()),
            false,
            seed.depth + 1,
            &cancel,
        );
        if outcome.canceled {
            return Err(ApiError::canceled());
        }
        if let Some(issue) = outcome.issues.first() {
            return Err(ApiError::internal(format!(
                "Auswahl unvollständig: {}: {}",
                issue.path, issue.detail
            )));
        }
        if !outcome.is_complete() {
            return Err(ApiError::internal("Auswahl unvollständig"));
        }
        files.extend(
            outcome
                .entries
                .into_iter()
                .filter(|entry| !entry.is_dir && compiled.matches(entry, base)),
        );
    }
    let snapshot =
        crate::filter::tree::clipboard_snapshot(files, base).map_err(ApiError::invalid)?;
    Ok(snapshot
        .into_iter()
        .map(|file| (file.abs, file.rel))
        .collect())
}
