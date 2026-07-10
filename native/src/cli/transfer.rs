use super::ops::{join, parent_of};
use super::os;
use super::target::Target;
use super::tree_guard::{validate_destination_state, validate_same_source, DestinationState};
use crate::vfs::VfsMeta;
use std::io;

pub(super) fn transfer_plan(src: &Target, dst: &Target) -> Result<(VfsMeta, String), String> {
    let meta = src.backend.stat(&src.path).map_err(|e| e.to_string())?;
    let dst_path = match dst.backend.stat(&dst.path) {
        Ok(destination) if destination.is_dir && !destination.is_symlink => {
            crate::vfs::validate_child_name(&meta.name).map_err(|error| error.to_string())?;
            join(&dst.path, &meta.name)
        }
        Ok(destination) if destination.is_dir => {
            return Err(format!(
                "destination directory is link-like and unsafe: {}",
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
    Ok((meta, dst_path))
}

/// Rename only when it preserves the copy+delete command semantics. An
/// existing directory must still use the recursive merge path, and an existing
/// file is replaced only when both `--force` and the backend's explicit
/// atomic-replace guarantee are present.
pub(super) fn try_rename_fast(
    src: &Target,
    dst: &Target,
    src_meta: &VfsMeta,
    dst_path: &str,
    force: bool,
) -> Result<bool, String> {
    if src_meta.is_dir {
        // Recursive moves need the same complete bounded source/destination
        // preflight as recursive copies. A direct rename would skip that tree
        // validation and could move a late link-like or ambiguous child.
        return Ok(false);
    }
    if !src.same_namespace(dst) || !src.can_rename_with(dst) {
        return Ok(false);
    }
    if src_meta.is_symlink {
        // The guarded transfer path rejects link-like sources. A rename would
        // move the link itself and bypass that fail-closed behavior.
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
    validate_same_source(
        src_meta,
        &src.backend
            .stat(&src.path)
            .map_err(|error| format!("source changed before rename {}: {error}", src.path))?,
        &src.path,
    )?;
    if let Some(parent) = parent_of(dst_path) {
        src.backend.mkdir_all(&parent).map_err(|e| e.to_string())?;
    }
    validate_same_source(
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
        Err(e)
            if src.backend.is_local()
                && (e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(17)) =>
        {
            // A local cross-volume move retains the established copy+delete
            // fallback. Other rename failures are surfaced because their remote
            // completion state may be ambiguous.
            Ok(false)
        }
        Err(e) => Err(format!("rename {} -> {}: {}", src.path, dst_path, e)),
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
