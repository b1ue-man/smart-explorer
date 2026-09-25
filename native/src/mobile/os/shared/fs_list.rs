//! `fs.list`, `fs.stat`, `fs.checkName` (api.md §4.3): one directory level
//! through the pooled backend, filtered and sorted like the desktop view.
use super::args::{bool_or, filter_arg, pass_all, sort_arg, str_arg};
use super::crumbs;
use super::entry::{entry_json, file_entry, problem_of, TreeInfo};
use super::error::ApiError;
use super::location::{join, parent_path, validate_name, zip_location, Loc, LocKind};
use super::runtime::Runtime;
use crate::filter::CompiledFilter;
use crate::types::FileEntry;
use crate::vfs::{DeleteDisposition, VfsMeta};
use serde_json::{json, Value};

pub(crate) fn list(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    if loc.kind == LocKind::Trash {
        return super::trash::listing();
    }
    // Opening a local `.zip` file browses the archive (read-only).
    let loc = if is_local_zip(&loc) {
        Loc::parse(&zip_location(&loc.path))?
    } else {
        loc
    };
    let show_hidden = bool_or(args, "showHidden", false);
    let filter = filter_arg(args, "filter", show_hidden)?.unwrap_or_else(|| pass_all(show_hidden));
    let compiled = CompiledFilter::compile(&filter);
    if let Some(error) = compiled.error() {
        return Err(ApiError::invalid(error.to_string()));
    }
    let sort = sort_arg(args, "sort")?;
    let (backend, _) = rt.resolve_loc(&loc)?;
    if bool_or(args, "refresh", false) {
        backend.invalidate_cache();
    }
    let metas = rt.with_read(&loc, |backend, path| backend.list_dir(path))?;
    let mut entries: Vec<FileEntry> = metas
        .iter()
        .filter(|meta| !crate::apptrash::excluded_name(&meta.name))
        .map(|meta| file_entry(&loc.path, meta, 1))
        .filter(|entry| compiled.matches(entry, &loc.path))
        .collect();
    entries.sort_by(|left, right| {
        crate::format::compare_entries(left, right, sort.key, sort.dir, sort.dirs_first)
    });
    let total_bytes: u64 = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();
    let rows: Vec<Value> = entries
        .iter()
        .map(|entry| entry_json(&loc, entry, TreeInfo::default()))
        .collect();
    let volumes = rt.volumes();
    let read_only = loc.kind == LocKind::Zip;
    let can_trash = match loc.kind {
        LocKind::Local => crumbs::volume_of(&loc.path, &volumes).is_some(),
        LocKind::Zip | LocKind::Trash => false,
        _ => backend.delete_disposition() == DeleteDisposition::Recycle,
    };
    let parent = if loc.kind == LocKind::Local && crumbs::is_volume_root(&loc.path, &volumes) {
        None
    } else {
        loc.parent()
    };
    super::places::add_recent(rt, &loc);
    Ok(json!({
        "location": loc.location(),
        "title": crumbs::title(&loc, &volumes),
        "crumbs": crumbs::crumbs(&loc, &volumes),
        "parent": parent,
        "backend": loc.kind.as_str(),
        "readOnly": read_only,
        "canTrash": can_trash,
        "entries": rows,
        "totalBytes": total_bytes,
    }))
}

fn is_local_zip(loc: &Loc) -> bool {
    loc.is_local()
        && loc.name().to_ascii_lowercase().ends_with(".zip")
        && std::fs::metadata(&loc.path).is_ok_and(|metadata| metadata.is_file())
}

/// The `Entry` of one location.
pub(crate) fn stat_entry(rt: &Runtime, loc: &Loc) -> Result<Value, ApiError> {
    let meta = rt.with_read(loc, |backend, path| backend.stat(path))?;
    Ok(meta_entry(loc, meta))
}

pub(crate) fn meta_entry(loc: &Loc, mut meta: VfsMeta) -> Value {
    if meta.name.is_empty() {
        meta.name = if loc.is_root() {
            crumbs::root_label(loc)
        } else {
            loc.name().to_string()
        };
    }
    let parent = parent_path(&loc.path).unwrap_or("/");
    let mut entry = file_entry(parent, &meta, 0);
    // A root keeps its own path rather than `/<label>`.
    entry.path = std::sync::Arc::from(loc.path.as_str());
    entry_json(loc, &entry, TreeInfo::default())
}

pub(crate) fn stat(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let loc = Loc::parse(str_arg(args, "location")?)?;
    reject_trash(&loc)?;
    stat_entry(rt, &loc)
}

pub(crate) fn check_name(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let parent = Loc::parse(str_arg(args, "parent")?)?;
    reject_trash(&parent)?;
    let name = str_arg(args, "name")?;
    let name = match validate_name(name) {
        Ok(name) => name,
        Err(error) => {
            return Ok(json!({ "problem": error.message, "exists": false, "invalid": true }))
        }
    };
    let path = join(&parent.path, name);
    let exists = rt.with_read(&parent, |backend, _| backend.try_exists(&path))?;
    Ok(json!({ "problem": problem_of(name), "exists": exists, "invalid": false }))
}

pub(crate) fn reject_trash(loc: &Loc) -> Result<(), ApiError> {
    if loc.kind == LocKind::Trash {
        return Err(ApiError::invalid(
            "Im Papierkorb bitte „Wiederherstellen“ oder „Endgültig löschen“ verwenden",
        ));
    }
    Ok(())
}

/// Rejects writes into read-only archives and the trash.
pub(crate) fn require_writable(loc: &Loc) -> Result<(), ApiError> {
    reject_trash(loc)?;
    if loc.kind == LocKind::Zip {
        return Err(ApiError::new(
            "permission",
            "ZIP ist schreibgeschützt — zum Bearbeiten entpacken",
        ));
    }
    Ok(())
}
