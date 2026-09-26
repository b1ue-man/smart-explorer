//! `fs.import` (shared content arriving as file descriptors) and `fs.extract`
//! (ZIP into a new folder) from api.md §4.3. Both write only new entries:
//! occupied names become `Name (2)`.
use super::args::{opt_str, str_arg};
use super::crumbs;
use super::drive::run_transfer;
use super::error::ApiError;
use super::fs_list::require_writable;
use super::location::{parent_path, validate_name, Loc, LocKind};
use super::runtime::{Runtime, TaskCtx};
use crate::transfer::{TransferMsg, TransferRequest};
use serde_json::{json, Value};
use std::fs::File;
use std::path::{Path, PathBuf};

struct Incoming {
    file: File,
    name: String,
    size: Option<u64>,
}

pub(crate) fn import(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let items = args
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("Argument „files“ fehlt"))?;
    // Adopt every descriptor first, so each one is closed even when a later
    // argument is rejected.
    let mut incoming = Vec::new();
    let mut rejected = None;
    let mut seen = std::collections::HashSet::new();
    for item in items {
        let Some(fd) = item.get("fd").and_then(Value::as_i64) else {
            rejected.get_or_insert_with(|| ApiError::invalid("Eintrag ohne „fd“"));
            continue;
        };
        // One descriptor must never be owned (and closed) twice.
        if !seen.insert(fd) {
            rejected.get_or_insert_with(|| ApiError::invalid("Doppelter Dateideskriptor"));
            continue;
        }
        match super::unix_fd::adopt(fd) {
            Ok(file) => incoming.push(Incoming {
                file,
                name: incoming_name(opt_str(item, "name").unwrap_or("")),
                size: item.get("size").and_then(Value::as_u64),
            }),
            Err(message) => {
                rejected.get_or_insert_with(|| ApiError::invalid(message));
            }
        }
    }
    if let Some(error) = rejected {
        return Err(error);
    }
    if incoming.is_empty() {
        return Err(ApiError::invalid("Keine Dateien empfangen"));
    }
    let target = Loc::parse(str_arg(args, "targetDir")?)?;
    require_writable(&target)?;
    let title = format!(
        "Empfangen: {} Datei(en) → {}",
        incoming.len(),
        crumbs::title(&target, &rt.volumes())
    );
    let runtime = rt.clone();
    let id = rt.spawn_transfer_task("upload", title, move |ctx| {
        run_import(&runtime, ctx, &target, incoming)
    });
    Ok(json!({ "taskId": id }))
}

/// A safe file name for shared content (providers may send anything).
fn incoming_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    match validate_name(base) {
        Ok(valid) => valid.to_string(),
        Err(_) => "Geteilte Datei".to_string(),
    }
}

fn run_import(
    rt: &Runtime,
    ctx: &TaskCtx,
    target: &Loc,
    incoming: Vec<Incoming>,
) -> Result<Value, ApiError> {
    let (backend, dest_dir) = rt.resolve_live(target)?;
    let count = incoming.len() as u64;
    let total: u64 = incoming.iter().filter_map(|item| item.size).sum();
    let cancel = ctx.cancel_flag();
    let mut done_bytes = 0u64;
    let mut names = Vec::new();
    let mut failed = 0u64;
    for (index, mut item) in incoming.into_iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let published = std::thread::scope(|scope| {
            let backend = &*backend;
            let file = &mut item.file;
            let (name, size) = (item.name.as_str(), item.size);
            let (dest_dir, cancel) = (dest_dir.as_str(), &*cancel);
            let worker = scope.spawn(move || {
                // Owning `tx` here ends the receive loop when the worker returns.
                let tx = tx;
                crate::transfer::upload_reader_progress(
                    backend, file, size, dest_dir, name, &tx, cancel,
                )
            });
            for message in rx.iter() {
                let (TransferMsg::Progress(progress) | TransferMsg::Done { progress, .. }) =
                    message;
                ctx.progress(
                    done_bytes.saturating_add(progress.bytes_done),
                    total,
                    index as u64,
                    count,
                );
            }
            worker
                .join()
                .unwrap_or_else(|_| Err("Interner Fehler beim Empfangen".to_string()))
        });
        match published {
            Ok(name) => {
                done_bytes = done_bytes.saturating_add(item.size.unwrap_or(0));
                names.push(name);
            }
            Err(_) if ctx.cancelled() => break,
            Err(error) => {
                failed += 1;
                ctx.error(&item.name, &error);
            }
        }
    }
    ctx.progress(done_bytes, total, names.len() as u64 + failed, count);
    let result = json!({ "files": names.len(), "names": names, "errors": failed });
    if ctx.cancelled() {
        return Err(ApiError::canceled());
    }
    if failed > 0 {
        ctx.set_failure_result(result);
        return Err(ApiError::internal(format!(
            "{failed} von {count} Datei(en) nicht gespeichert"
        )));
    }
    Ok(result)
}

pub(crate) fn extract(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let archive = Loc::parse(str_arg(args, "location")?)?;
    if archive.kind != LocKind::Local {
        return Err(ApiError::unsupported(
            "Entpacken geht nur für Archive im Gerätespeicher – bitte zuerst herunterladen",
        ));
    }
    let target = match opt_str(args, "targetDir") {
        Some(dir) => Loc::parse(dir)?,
        None => Loc::parse(parent_path(&archive.path).unwrap_or("/"))?,
    };
    require_writable(&target)?;
    let title = format!("Entpacken: {}", archive.name());
    let runtime = rt.clone();
    let id = rt.spawn_transfer_task("extract", title, move |ctx| {
        run_extract(&runtime, ctx, &archive, &target)
    });
    Ok(json!({ "taskId": id }))
}

/// The folder name for an archive: its name without `.zip`.
fn folder_name(archive: &Loc) -> String {
    let name = archive.name();
    let stem = match name.rfind('.') {
        Some(index) if index > 0 => &name[..index],
        _ => name,
    };
    if stem.trim().is_empty() {
        "Archiv".to_string()
    } else {
        stem.to_string()
    }
}

/// Creates `parent/name`, `parent/name (2)`, … and returns the new folder.
fn create_unique_dir(parent: &Path, name: &str) -> Result<PathBuf, ApiError> {
    for index in 1..=crate::vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS {
        let candidate = parent.join(crate::vfs::remote_util::numbered_remote_name(name, index));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ApiError::from(error).context("Zielordner anlegen")),
        }
    }
    Err(ApiError::new("exists", "Kein freier Ordnername gefunden"))
}

fn run_extract(
    rt: &Runtime,
    ctx: &TaskCtx,
    archive: &Loc,
    target: &Loc,
) -> Result<Value, ApiError> {
    let name = folder_name(archive);
    if target.is_local() {
        let dest = create_unique_dir(Path::new(&target.path), &name)?;
        let report = extract_into(ctx, &archive.path, &dest);
        let dest_text = dest.to_string_lossy().into_owned();
        return match report {
            Ok(report) if report.canceled || ctx.cancelled() => {
                let _ = crate::transfer::remove_owned_tree(Path::new(&target.path), &dest);
                Err(ApiError::canceled())
            }
            Ok(report) => finish_extract(ctx, report, json!(dest_text)),
            Err(error) => {
                let _ = crate::transfer::remove_owned_tree(Path::new(&target.path), &dest);
                Err(error)
            }
        };
    }
    // Remote target: extract into a private temp folder, then upload it.
    let temp_root = crate::support_dirs::temp_dir();
    let staging = create_unique_dir(&temp_root, &format!("extract-{}", ctx.id()))?;
    let result = (|| -> Result<Value, ApiError> {
        let dest = staging.join(&name);
        std::fs::create_dir(&dest)?;
        ctx.message("Entpacke…");
        let report = extract_into(ctx, &archive.path, &dest)?;
        if report.canceled || ctx.cancelled() {
            return Err(ApiError::canceled());
        }
        ctx.message("Lade hoch…");
        let (backend, dest_root) = rt.resolve_live(target)?;
        let outcome = run_transfer(
            ctx,
            TransferRequest::Upload {
                paths: vec![dest.to_string_lossy().into_owned()],
                backend,
                dest_root,
            },
        )?;
        outcome.into_result(ctx)?;
        // The upload may number the folder; the result names its parent.
        finish_extract(ctx, report, json!(target.location()))
    })();
    let _ = crate::transfer::remove_owned_tree(&temp_root, &staging);
    result
}

fn extract_into(
    ctx: &TaskCtx,
    archive: &str,
    dest: &Path,
) -> Result<crate::zipfs::ExtractReport, ApiError> {
    let cancel = ctx.cancel_flag();
    crate::zipfs::extract_all_controlled(archive, dest, &cancel, |progress| {
        ctx.progress(
            progress.bytes_done,
            progress.bytes_total,
            progress.files_done,
            progress.files_total,
        )
    })
    .map_err(|error| ApiError::from(error).context("Archiv lesen"))
}

fn finish_extract(
    ctx: &TaskCtx,
    report: crate::zipfs::ExtractReport,
    location: Value,
) -> Result<Value, ApiError> {
    for (path, reason) in &report.skipped {
        ctx.error(path, reason);
    }
    let result = json!({
        "location": location,
        "files": report.files,
        "bytes": report.bytes,
        "skipped": report.skipped.len(),
    });
    if !report.skipped.is_empty() {
        ctx.message(&format!("{} Einträge nicht entpackt", report.skipped.len()));
    }
    Ok(result)
}
