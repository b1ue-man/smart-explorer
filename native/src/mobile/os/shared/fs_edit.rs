//! `fs.mkdir`, `fs.newFile`, `fs.rename`, `fs.conflicts` (api.md §4.3).
//! Nothing here replaces an existing entry: an occupied name is `exists`.
use super::args::{nonempty_list, str_arg};
use super::error::ApiError;
use super::fs_list::{meta_entry, require_writable, stat_entry};
use super::location::{join, parent_path, validate_name, Loc};
use super::runtime::Runtime;
use crate::vfs::Backend;
use serde_json::{json, Value};
use std::io::{self, Write};

fn exists_error(name: &str) -> ApiError {
    ApiError::new("exists", format!("„{name}“ existiert bereits"))
}

fn target(args: &Value) -> Result<(Loc, String, String), ApiError> {
    let parent = Loc::parse(str_arg(args, "parent")?)?;
    require_writable(&parent)?;
    let name = validate_name(str_arg(args, "name")?)?.to_string();
    let path = join(&parent.path, &name);
    Ok((parent, name, path))
}

pub(crate) fn mkdir(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let (parent, name, path) = target(args)?;
    let (backend, _) = rt.resolve_loc(&parent)?;
    if parent.is_local() {
        std::fs::create_dir(&path).map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => exists_error(&name),
            _ => ApiError::from(error).context("Ordner anlegen"),
        })?;
    } else {
        if backend.try_exists(&path)? {
            return Err(exists_error(&name));
        }
        backend
            .mkdir_all(&path)
            .map_err(|error| ApiError::from(error).context("Ordner anlegen"))?;
    }
    created_entry(&*backend, &parent, &path)
}

pub(crate) fn new_file(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let (parent, name, path) = target(args)?;
    let (backend, _) = rt.resolve_loc(&parent)?;
    if parent.is_local() {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => exists_error(&name),
                _ => ApiError::from(error).context("Datei anlegen"),
            })?;
    } else {
        if backend.try_exists(&path)? {
            return Err(exists_error(&name));
        }
        create_remote_empty(&*backend, &path).map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => exists_error(&name),
            _ => ApiError::from(error).context("Datei anlegen"),
        })?;
    }
    created_entry(&*backend, &parent, &path)
}

/// An empty remote file, published create-only from a private stage.
fn create_remote_empty(backend: &dyn Backend, path: &str) -> io::Result<()> {
    let staged = crate::vfs::unique_staging_path(backend, path, "new-file")?;
    let result = (|| {
        let mut writer = backend.open_write(&staged)?;
        writer.flush()?;
        drop(writer);
        crate::vfs::promote_staged_create(backend, &staged, path)
    })();
    if result.is_err() {
        let _ = backend.remove_file(&staged);
    }
    result
}

fn created_entry(backend: &dyn Backend, parent: &Loc, path: &str) -> Result<Value, ApiError> {
    let loc = Loc::parse(&parent.at(path))?;
    let meta = backend.stat(path)?;
    Ok(meta_entry(&loc, meta))
}

pub(crate) fn rename(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    require_writable(&loc)?;
    if loc.is_root() {
        return Err(ApiError::invalid("Die Wurzel kann nicht umbenannt werden"));
    }
    let name = validate_name(str_arg(args, "newName")?)?;
    if name == loc.name() {
        return stat_entry(rt, &loc);
    }
    let parent = parent_path(&loc.path).unwrap_or("/");
    let destination = join(parent, name);
    let (backend, _) = rt.resolve_loc(&loc)?;
    backend
        .rename_no_replace(&loc.path, &destination)
        .map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => exists_error(name),
            _ => ApiError::from(error).context("Umbenennen"),
        })?;
    let renamed = Loc::parse(&loc.at(&destination))?;
    let meta = backend.stat(&destination)?;
    Ok(meta_entry(&renamed, meta))
}

/// Names of `sources` that already exist in `targetDir`.
pub(crate) fn conflicts(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let sources = nonempty_list(args, "sources")?;
    let target = Loc::parse(str_arg(args, "targetDir")?)?;
    require_writable(&target)?;
    let mut all_local = target.is_local();
    let mut names = Vec::new();
    for source in &sources {
        let source = Loc::parse(source)?;
        all_local &= source.is_local();
        let name = source.name().to_string();
        if name.is_empty() || names.contains(&name) {
            continue;
        }
        let path = join(&target.path, &name);
        if rt.with_read(&target, |backend, _| backend.try_exists(&path))? {
            names.push(name);
        }
    }
    Ok(json!({ "names": names, "choosable": all_local }))
}
