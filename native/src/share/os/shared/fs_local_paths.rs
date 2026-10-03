//! Host target resolution behind the Share local-path compatibility surface.
use std::{io, path::Path};

use super::{
    core::eio,
    fs_path_adapter::{canonical_contains, from_os_path},
    fs_paths::norm_root,
};

pub(in crate::share) use super::fs_path_adapter::to_os_path;

pub(super) fn secure_local_target(root: &str, rest: &[String]) -> io::Result<String> {
    let root_norm = norm_root(root);
    let root_os = to_os_path(&root_norm);
    let root_canon = std::fs::canonicalize(&root_os).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Freigabe-Wurzel kann nicht gelesen werden: {error}"),
        )
    })?;
    let target_os = rest
        .iter()
        .fold(root_canon.clone(), |p, segment| p.join(segment));
    ensure_under_root(&root_canon, &target_os)?;
    Ok(from_os_path(&target_os))
}

fn ensure_under_root(root_canon: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        let target_canon = std::fs::canonicalize(target)?;
        if !canonical_contains(root_canon, &target_canon) {
            return Err(eio("Symlink/Reparse-Point fuehrt aus der Freigabe heraus"));
        }
        return Ok(());
    }

    let mut existing = target;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| eio("Ziel hat keinen gueltigen Elternordner"))?;
    }
    let existing_canon = std::fs::canonicalize(existing)?;
    if !canonical_contains(root_canon, &existing_canon) {
        return Err(eio("Ziel liegt ausserhalb der Freigabe"));
    }
    Ok(())
}
