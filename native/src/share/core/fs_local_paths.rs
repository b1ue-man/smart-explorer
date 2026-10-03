use std::{io, path::{Path, PathBuf}};
use super::core::eio;
use super::fs_paths::norm_root;
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
        if !target_canon.starts_with(root_canon) {
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
    if !existing_canon.starts_with(root_canon) {
        return Err(eio("Ziel liegt ausserhalb der Freigabe"));
    }
    Ok(())
}

pub(in crate::share) fn to_os_path(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    if std::path::MAIN_SEPARATOR == '/' {
        PathBuf::from(path)
    } else {
        PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

fn from_os_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
