use super::ops::{join, parent_of};
use super::os;
use super::target::Target;
use super::tree_guard::{validate_destination_state, validate_same_source, DestinationState};
use crate::vfs::VfsMeta;
use std::io;

pub(super) fn transfer_plan(src: &Target, dst: &Target) -> Result<(VfsMeta, String), String> {
    transfer_plan_as(src, dst, None)
}

/// [`transfer_plan`] for a source copied under another name (a resolved
/// link keeps the link's name inside a destination directory).
pub(super) fn transfer_plan_as(
    src: &Target,
    dst: &Target,
    name: Option<&str>,
) -> Result<(VfsMeta, String), String> {
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let name = name.unwrap_or(&meta.name);
    let dst_path = match dst.backend.stat(&dst.path) {
        Ok(destination) if destination.is_dir && !destination.is_symlink => {
            crate::vfs::validate_child_name(name).map_err(|error| error.to_string())?;
            join(&dst.path, name)
        }
        Ok(destination) if destination.is_dir => {
            return Err(format!(
                "destination {} is a link; Smart Explorer writes through links only on local paths. Name the directory it points to instead.",
                dst.path
            ));
        }
        Ok(_) => dst.path.clone(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => dst.path.clone(),
        Err(error) => {
            return Err(format!(
                "cannot determine destination type for {}: {error}",
                dst.path
            ))
        }
    };
    validate_transfer_path(src, dst, &meta, &dst_path)?;
    Ok((meta, dst_path))
}

pub(super) fn validate_transfer_path(
    src: &Target,
    dst: &Target,
    meta: &VfsMeta,
    dst_path: &str,
) -> Result<(), String> {
    if src.same_namespace(dst) {
        let src_cmp = comparable_path(src, &src.path)?;
        let dst_cmp = comparable_path(dst, &dst_path)?;
        let same_local_file = if src_cmp != dst_cmp
            && !meta.is_dir
            && src.backend.is_local()
            && dst.backend.is_local()
        {
            match os::same_file(&src.path, &dst_path) {
                Ok(same) => same,
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                // A link to a directory (Windows refuses to open a junction
                // as a file) or a dangling link leads to no file that could
                // be the destination; the link itself is what moves.
                Err(_) if meta.is_symlink => false,
                Err(e) => {
                    return Err(format!(
                        "cannot verify that local source and destination differ: {e}"
                    ))
                }
            }
        } else {
            false
        };
        if src_cmp == dst_cmp || same_local_file {
            return Err(format!(
                "source and destination are the same path or file: {} -> {}",
                src.path, dst_path
            ));
        }
        if meta.is_dir && dst_cmp.is_descendant_of(&src_cmp) {
            return Err(format!(
                "cannot copy or move directory into its own descendant: {} -> {}",
                src.path, dst_path
            ));
        }
    }
    Ok(())
}

/// Rename only when it preserves the copy+delete command semantics. An
/// existing directory must still use the recursive merge path, and an existing
/// file is replaced only when both `--force` and the backend's explicit
/// atomic-replace guarantee are present. A directory or link renamed to a new
/// name moves as it is: links inside stay links (nothing is read or
/// followed), so no tree preflight is needed. `Ok(false)` means nothing was
/// renamed and the copy+delete path decides.
pub(super) fn try_rename_fast(
    src: &Target,
    dst: &Target,
    src_meta: &VfsMeta,
    dst_path: &str,
    force: bool,
) -> Result<bool, String> {
    if !src.same_namespace(dst) || !src.can_rename_with(dst) {
        return Ok(false);
    }
    let destination_state = match dst.backend.stat(dst_path) {
        Ok(existing) => {
            if !(force
                && !src_meta.is_dir
                && !existing.is_dir
                && !existing.is_symlink
                && src.backend.rename_overwrites())
            {
                return Ok(false);
            }
            DestinationState::File(existing)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => DestinationState::Missing,
        // Several network backends collapse protocol status into Other. Do not
        // assume that means "absent" and accidentally overwrite an unknown item.
        Err(_) => return Ok(false),
    };
    validate_rename_source(
        src_meta,
        &src.backend
            .stat(&src.path)
            .map_err(|error| format!("source changed before rename {}: {error}", src.path))?,
        &src.path,
    )?;
    if let Some(parent) = parent_of(dst_path) {
        src.backend.mkdir_all(&parent).map_err(|e| e.to_string())?;
    }
    validate_rename_source(
        src_meta,
        &src.backend.stat(&src.path).map_err(|error| {
            format!(
                "source changed immediately before rename {}: {error}",
                src.path
            )
        })?,
        &src.path,
    )?;
    validate_destination_state(&*dst.backend, dst_path, &destination_state, false)?;
    let renamed = if matches!(&destination_state, DestinationState::Missing) {
        src.backend.rename_no_replace(&src.path, dst_path)
    } else {
        src.backend.rename(&src.path, dst_path)
    };
    match renamed {
        Ok(()) => Ok(true),
        // The backend (FTP, Google Drive, ZIP) or the filesystem cannot
        // rename without replacing: nothing was renamed, copy+delete moves it.
        Err(e) if e.kind() == io::ErrorKind::Unsupported => Ok(false),
        Err(e) if src.backend.is_local() && os::crosses_devices(&e) => {
            // A local cross-volume move retains the established copy+delete
            // fallback. Other rename failures are surfaced because their remote
            // completion state may be ambiguous.
            Ok(false)
        }
        Err(e) => Err(format!("rename {} -> {}: {}", src.path, dst_path, e)),
    }
}

/// Attempts for the unique intermediate name of a case rename.
const CASE_RENAME_ATTEMPTS: usize = 8;

/// `se mv` to the same name in another case in the same folder
/// (`STICK/foto.jpg` → `STICK/Foto.jpg`, U10). Where the filesystem folds
/// names (FAT, exFAT, NTFS) the destination is the source itself, and a
/// rename onto itself changes nothing (Linux vfat), so the entry is renamed
/// in two steps through a unique name beside it; neither step replaces
/// anything, and a failed second step renames it back. A folder needs
/// `recursive` as for every move. `Ok(false)`: not this case, nothing was
/// touched.
pub(super) fn try_case_rename(src: &Target, dst: &Target, recursive: bool) -> Result<bool, String> {
    if !src.backend.is_local()
        || !dst.backend.is_local()
        || !src.same_namespace(dst)
        || !src.can_rename_with(dst)
    {
        return Ok(false);
    }
    let (Some(folder), Some(other)) = (parent_of(&src.path), parent_of(&dst.path)) else {
        return Ok(false);
    };
    let (from, to) = (leaf(&src.path), leaf(&dst.path));
    if folder != other || from.is_empty() || from == to || from.to_lowercase() != to.to_lowercase()
    {
        return Ok(false);
    }
    let Ok(source) = src.backend.stat(&src.path) else {
        return Ok(false);
    };
    if source.is_symlink || (source.is_dir && !recursive) || !same_entry(src, dst, &source)? {
        return Ok(false);
    }
    for _ in 0..CASE_RENAME_ATTEMPTS {
        let id = crate::transfer::new_job_id();
        let intermediate = join(&folder, &format!(".se-case-{}", &id[..16]));
        match src.backend.rename_no_replace(&src.path, &intermediate) {
            Ok(()) => return finish_case_rename(src, &intermediate, &dst.path).map(|()| true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            // Nothing was renamed: the ordinary path decides.
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(false),
            Err(error) => return Err(format!("rename {} -> {intermediate}: {error}", src.path)),
        }
    }
    Err(format!(
        "rename {}: no unique intermediate name for the change of case",
        src.path
    ))
}

/// Whether `dst` names the very entry `src` names (same object through a
/// name that differs in case only), never a link or another entry.
fn same_entry(src: &Target, dst: &Target, source: &VfsMeta) -> Result<bool, String> {
    let Ok(found) = dst.backend.stat(&dst.path) else {
        return Ok(false);
    };
    if found.is_symlink || found.is_dir != source.is_dir {
        return Ok(false);
    }
    match os::same_file(&src.path, &dst.path) {
        Ok(same) => Ok(same),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        // Windows opens no folder as a file: the stored name the listing
        // reports and the folder's times decide.
        Err(_) if source.is_dir => Ok(found.name == source.name
            && found.mtime_ms == source.mtime_ms
            && found.btime_ms == source.btime_ms),
        Err(error) => Err(format!(
            "cannot verify that local source and destination differ: {error}"
        )),
    }
}

fn finish_case_rename(src: &Target, intermediate: &str, destination: &str) -> Result<(), String> {
    let Err(error) = src.backend.rename_no_replace(intermediate, destination) else {
        return Ok(());
    };
    match src.backend.rename_no_replace(intermediate, &src.path) {
        Ok(()) => Err(format!("rename {} -> {destination}: {error}", src.path)),
        Err(back) => Err(format!(
            "rename {} -> {destination}: {error}; the entry is now named {intermediate} ({back})",
            src.path
        )),
    }
}

fn leaf(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// A link is renamed as itself; everything else must be the planned entry.
fn validate_rename_source(expected: &VfsMeta, actual: &VfsMeta, path: &str) -> Result<(), String> {
    if !expected.is_symlink {
        return validate_same_source(expected, actual, path);
    }
    if actual.is_symlink
        && actual.name == expected.name
        && actual.id == expected.id
        && actual.size == expected.size
        && actual.mtime_ms == expected.mtime_ms
    {
        Ok(())
    } else {
        Err(format!("source link changed before rename: {path}"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComparablePath {
    root: String,
    parts: Vec<String>,
}

impl ComparablePath {
    fn is_descendant_of(&self, parent: &Self) -> bool {
        self.root == parent.root
            && self.parts.len() > parent.parts.len()
            && self.parts.starts_with(&parent.parts)
    }
}

fn comparable_path(target: &Target, path: &str) -> Result<ComparablePath, String> {
    if !target.backend.is_local() {
        return Ok(normalize_path(path, false));
    }
    let resolved = resolve_local_path(path)
        .map_err(|e| format!("cannot resolve local path for a safe transfer: {path}: {e}"))?;
    Ok(normalize_path(&resolved, cfg!(windows)))
}

/// Resolve symlinked existing ancestors for local destinations, including a
/// destination leaf that does not exist yet. This closes aliases such as
/// `source/link-to-source/new` that a lexical descendant check would miss.
fn resolve_local_path(path: &str) -> io::Result<String> {
    let native = os::local_path(path);
    let mut probe = if native.is_absolute() {
        native
    } else {
        std::env::current_dir()?.join(native)
    };
    let mut missing = Vec::new();
    while !probe.try_exists()? {
        let name = probe.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor for {}", probe.display()),
            )
        })?;
        missing.push(name.to_os_string());
        probe = probe
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "path has no parent"))?
            .to_path_buf();
    }
    let mut resolved = std::fs::canonicalize(probe)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved.to_string_lossy().replace('\\', "/"))
}

fn normalize_path(path: &str, case_insensitive: bool) -> ComparablePath {
    let path = path.trim().replace('\\', "/");
    let (mut root, rest) = if let Some(rest) = path.strip_prefix("//") {
        ("//".to_string(), rest)
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/".to_string(), rest)
    } else if path.as_bytes().get(1) == Some(&b':') {
        (path[..2].to_string(), path[2..].trim_start_matches('/'))
    } else {
        (String::new(), path.as_str())
    };
    let mut parts: Vec<String> = Vec::new();
    for part in rest
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if part == ".." {
            if parts.last().is_some_and(|last| last != "..") {
                parts.pop();
            } else if root.is_empty() {
                parts.push(part.to_string());
            }
        } else {
            parts.push(part.to_string());
        }
    }
    if case_insensitive {
        root = root.to_lowercase();
        for part in &mut parts {
            *part = part.to_lowercase();
        }
    }
    ComparablePath { root, parts }
}

#[cfg(test)]
#[path = "transfer_case_tests.rs"]
mod case_tests;

#[cfg(test)]
mod tests {
    use super::normalize_path;

    #[test]
    fn normalized_paths_use_component_boundaries_for_descendants() {
        let parent = normalize_path("/srv/data", false);
        assert!(normalize_path("/srv/data/sub", false).is_descendant_of(&parent));
        assert!(!normalize_path("/srv/database", false).is_descendant_of(&parent));
        assert_eq!(normalize_path("/srv/data/./x/..", false), parent);
    }
}
