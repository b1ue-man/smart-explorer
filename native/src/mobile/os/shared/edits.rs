//! `fs.open`, `fs.fetch`, `fs.materialize`, `fs.edits`, `fs.uploadEdit`,
//! `fs.discardEdit` (api.md §4.3). Remote files open from a downloaded copy
//! under `<cache>/open/<editId>/`; the register survives restarts, and a
//! changed copy is announced with an `edits` event.
use super::args::{bool_or, nonempty_list, opt_str, str_arg};
use super::drive::run_transfer;
use super::edits_store::{self as store, EditRecord, MAX_EDITS};
use super::entry::mime_of;
use super::error::ApiError;
use super::fs_list::reject_trash;
use super::location::{parent_path, Loc};
use super::runtime::{Runtime, TaskCtx};
use crate::transfer::TransferRequest;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Copy folders without a register entry are removed once this old.
const ORPHAN_MIN_AGE: Duration = Duration::from_secs(10 * 60);

pub(crate) fn handle(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "fs.open" => open(rt, args),
        "fs.fetch" => fetch(rt, args),
        "fs.materialize" => materialize(rt, args),
        "fs.edits" => Ok(edits(rt)),
        "fs.uploadEdit" => upload_edit(rt, args),
        "fs.discardEdit" => discard_edit(rt, args),
        _ => return None,
    })
}

fn open(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    if !loc.is_local() {
        return Err(ApiError::unsupported(
            "Remote-Dateien werden vor dem Öffnen geladen (fs.fetch)",
        ));
    }
    let meta = rt.with_read(&loc, |backend, path| backend.stat(path))?;
    if meta.is_dir {
        return Err(ApiError::invalid(
            "Ordner werden nicht mit einer App geöffnet",
        ));
    }
    Ok(json!({ "localPath": loc.path, "mime": mime_of(loc.name()) }))
}

fn fetch(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&loc)?;
    if loc.is_local() {
        return Err(ApiError::invalid("Lokale Dateien direkt öffnen (fs.open)"));
    }
    if store::load(rt).len() >= MAX_EDITS {
        return Err(ApiError::new(
            "busy",
            "Zu viele geöffnete Remote-Dateien – bitte erst hochladen oder verwerfen",
        ));
    }
    let title = format!("Laden: {}", loc.name());
    let runtime = rt.clone();
    let id = rt.spawn_task("open", title, move |ctx| run_fetch(&runtime, ctx, &loc));
    Ok(json!({ "taskId": id }))
}

/// Downloads one remote file into a fresh folder below `root`.
fn download_one(
    rt: &Runtime,
    ctx: &TaskCtx,
    loc: &Loc,
    root: &Path,
) -> Result<(PathBuf, PathBuf, i64), ApiError> {
    let meta = rt.with_read(loc, |backend, path| backend.stat(path))?;
    if meta.is_dir {
        return Err(ApiError::invalid(format!(
            "„{}“ ist ein Ordner",
            loc.name()
        )));
    }
    let (backend, path) = rt.resolve_loc(loc)?;
    let dir = root.join(store::new_id());
    std::fs::create_dir_all(&dir)
        .map_err(|error| ApiError::from(error).context("Zwischenspeicher anlegen"))?;
    let result = (|| -> Result<PathBuf, ApiError> {
        run_transfer(
            ctx,
            TransferRequest::Download {
                backend,
                files: vec![path],
                dest_local: dir.to_string_lossy().into_owned(),
                filter: None,
            },
        )?
        .into_result(ctx)?;
        single_file(&dir)
    })();
    match result {
        Ok(file) => Ok((dir, file, meta.mtime_ms)),
        Err(error) => {
            let _ = crate::transfer::remove_owned_tree(root, &dir);
            Err(error)
        }
    }
}

fn single_file(dir: &Path) -> Result<PathBuf, ApiError> {
    std::fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_file())
        .ok_or_else(|| ApiError::internal("Die geladene Datei fehlt"))
}

fn run_fetch(rt: &Runtime, ctx: &TaskCtx, loc: &Loc) -> Result<Value, ApiError> {
    let root = store::open_root(rt);
    let (dir, file, remote_mtime) = download_one(rt, ctx, loc, &root)?;
    let edit_id = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| loc.name().to_string());
    let mut record = EditRecord {
        edit_id: edit_id.clone(),
        name: name.clone(),
        location: loc.location(),
        local_path: file.to_string_lossy().into_owned(),
        remote_mtime_ms: remote_mtime,
        ..EditRecord::default()
    };
    record.rebase_local();
    store::update(rt, |records| records.push(record));
    Ok(json!({
        "localPath": file.to_string_lossy(),
        "mime": mime_of(&name),
        "editId": edit_id,
    }))
}

fn materialize(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let mut locations = Vec::new();
    for location in nonempty_list(args, "locations")? {
        let loc = Loc::parse(&location)?;
        reject_trash(&loc)?;
        locations.push(loc);
    }
    let title = format!("Zum Teilen vorbereiten: {} Element(e)", locations.len());
    let runtime = rt.clone();
    let id = rt.spawn_task("materialize", title, move |ctx| {
        let root = runtime.config().cache_subdir("share");
        let mut paths = Vec::new();
        for (index, loc) in locations.iter().enumerate() {
            if ctx.cancelled() {
                return Err(ApiError::canceled());
            }
            if loc.is_local() {
                paths.push(loc.path.clone());
            } else {
                let (_, file, _) = download_one(&runtime, ctx, loc, &root)?;
                paths.push(file.to_string_lossy().into_owned());
            }
            ctx.progress(0, 0, index as u64 + 1, locations.len() as u64);
        }
        Ok(json!({ "paths": paths }))
    });
    Ok(json!({ "taskId": id }))
}

/// The register with a `modified` flag; copies that vanished are dropped and
/// newly changed ones announced once with an `edits` event.
fn edits(rt: &Runtime) -> Value {
    let (listing, announce) = store::update(rt, |records| {
        records.retain(|record| record.modified().is_some());
        let mut announce = false;
        let listing: Vec<Value> = records
            .iter_mut()
            .map(|record| {
                let modified = record.modified().unwrap_or(false);
                if modified && !record.notified {
                    record.notified = true;
                    announce = true;
                }
                json!({
                    "editId": record.edit_id,
                    "name": record.name,
                    "location": record.location,
                    "localPath": record.local_path,
                    "modified": modified,
                })
            })
            .collect();
        (listing, announce)
    });
    if announce {
        rt.emit(json!({ "type": "edits" }));
    }
    Value::Array(listing)
}

/// At start: forget copies that vanished, remove orphaned copy folders and
/// announce changed copies.
pub(crate) fn check_on_start(rt: &Runtime) {
    let (known, changed) = store::update(rt, |records| {
        records.retain(|record| record.modified().is_some());
        let changed = records.iter().any(|record| record.modified() == Some(true));
        let known: Vec<String> = records
            .iter()
            .map(|record| record.edit_id.clone())
            .collect();
        (known, changed)
    });
    let root = store::open_root(rt);
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A download that has just started is not registered yet.
            let settled = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > ORPHAN_MIN_AGE);
            if known.contains(&name) || !settled {
                continue;
            }
            if let Err(error) = crate::transfer::remove_owned_tree(&root, &entry.path()) {
                rt.record_error("Alte Kopien aufräumen", &error.to_string());
            }
        }
    }
    if changed {
        rt.emit(json!({ "type": "edits" }));
    }
}

fn find(rt: &Runtime, edit_id: &str) -> Result<EditRecord, ApiError> {
    store::load(rt)
        .into_iter()
        .find(|record| record.edit_id == edit_id)
        .ok_or_else(|| ApiError::not_found("Diese geöffnete Datei ist nicht mehr registriert"))
}

fn upload_edit(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let record = find(rt, str_arg(args, "editId")?)?;
    let overwrite = match opt_str(args, "mode").unwrap_or("overwrite") {
        "overwrite" => true,
        "copy" => false,
        other => return Err(ApiError::invalid(format!("Unbekannter Modus: {other}"))),
    };
    // After a reported conflict the user may explicitly overwrite the newer remote file,
    // like saving again after the desktop's conflict notice.
    let force = bool_or(args, "force", false);
    let loc = Loc::parse(&record.location)?;
    let title = format!("Hochladen: {}", record.name);
    let runtime = rt.clone();
    let id = rt.spawn_task("upload", title, move |ctx| {
        if overwrite {
            run_overwrite(&runtime, ctx, &loc, record, force)
        } else {
            run_upload_copy(&runtime, ctx, &loc, record)
        }
    });
    Ok(json!({ "taskId": id }))
}

fn run_overwrite(
    rt: &Runtime,
    ctx: &TaskCtx,
    loc: &Loc,
    record: EditRecord,
    force: bool,
) -> Result<Value, ApiError> {
    let (backend, path) = rt.resolve_loc(loc)?;
    // Fresh metadata: the pooled backend may answer stat from a cached listing.
    let current = crate::vfs::sync_backend(backend.clone()).stat(&path)?;
    if !force && record.remote_mtime_ms != 0 && current.mtime_ms > record.remote_mtime_ms {
        ctx.set_failure_result(json!({ "conflict": true }));
        return Err(ApiError::new(
            "conflict",
            "Die Remote-Datei wurde seit dem Öffnen geändert",
        ));
    }
    ctx.message("Lade hoch…");
    crate::transfer::upload_file(&*backend, Path::new(&record.local_path), &path)
        .map_err(ApiError::internal)?;
    let remote_mtime = backend.stat(&path).map(|meta| meta.mtime_ms).unwrap_or(0);
    rebase(rt, &record.edit_id, Some(remote_mtime));
    Ok(json!({ "location": loc.location() }))
}

fn run_upload_copy(
    rt: &Runtime,
    ctx: &TaskCtx,
    loc: &Loc,
    record: EditRecord,
) -> Result<Value, ApiError> {
    let (backend, _) = rt.resolve_loc(loc)?;
    let parent = parent_path(&loc.path).unwrap_or("/").to_string();
    run_transfer(
        ctx,
        TransferRequest::Upload {
            paths: vec![record.local_path.clone()],
            backend,
            dest_root: parent.clone(),
        },
    )?
    .into_result(ctx)?;
    rebase(rt, &record.edit_id, None);
    Ok(json!({ "location": loc.at(&parent) }))
}

/// The local copy's current state becomes the baseline after an upload.
fn rebase(rt: &Runtime, edit_id: &str, remote_mtime: Option<i64>) {
    store::update(rt, |records| {
        if let Some(record) = records.iter_mut().find(|record| record.edit_id == edit_id) {
            record.rebase_local();
            if let Some(mtime) = remote_mtime {
                record.remote_mtime_ms = mtime;
            }
        }
    });
}

fn discard_edit(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let edit_id = str_arg(args, "editId")?.to_string();
    let removed = store::update(rt, |records| {
        let before = records.len();
        records.retain(|record| record.edit_id != edit_id);
        before != records.len()
    });
    let root = store::open_root(rt);
    let dir = root.join(&edit_id);
    if removed && dir.exists() {
        crate::transfer::remove_owned_tree(&root, &dir)
            .map_err(|error| ApiError::from(error).context("Kopie löschen"))?;
    }
    Ok(json!({}))
}
