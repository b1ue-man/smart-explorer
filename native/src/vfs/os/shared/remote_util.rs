//! Small backend-path helpers shared by the GUI, transfers and sync merges:
//! path joins, numbered and conflict names, unique-name probing, and bounded
//! text reads/replacing writes for the line merge.
use super::{Backend, VfsResult};

const MAX_MERGE_TEXT_BYTES: u64 = 16 * 1024 * 1024;

/// Upper bound for numbered-name probing (`name`, `name (2)`, …).
pub const REMOTE_UNIQUE_ATTEMPTS: usize = 1000;

pub fn ep_join(root: &str, rel: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), rel)
}

/// Insert " (Konflikt <timestamp>)" before the extension of a relative path.
pub fn conflict_rel_name(rel: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let seg_start = rel.rfind('/').map(|i| i + 1).unwrap_or(0);
    match rel[seg_start..].rfind('.') {
        Some(d) => {
            let dot = seg_start + d;
            format!("{} (Konflikt {}){}", &rel[..dot], ts, &rel[dot..])
        }
        None => format!("{} (Konflikt {})", rel, ts),
    }
}

pub fn numbered_remote_name(name: &str, index: usize) -> String {
    if index <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({index}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({index})"),
    }
}

/// Read a remote file as UTF-8 text (errors on binary), for the line-merge view.
pub fn read_text(be: &dyn Backend, path: &str) -> Result<String, String> {
    use std::io::Read;
    let metadata = be.stat(path).map_err(|error| error.to_string())?;
    if metadata.is_dir || metadata.is_symlink {
        return Err("Nur reguläre Dateien können als Text zusammengeführt werden.".to_string());
    }
    if metadata.size > MAX_MERGE_TEXT_BYTES {
        return Err("Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.".to_string());
    }
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.by_ref()
        .take(MAX_MERGE_TEXT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_MERGE_TEXT_BYTES {
        return Err("Text-Zusammenführung ist auf 16 MiB pro Datei begrenzt.".to_string());
    }
    if buf.contains(&0) {
        return Err("Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string());
    }
    String::from_utf8(buf)
        .map_err(|_| "Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string())
}

pub fn write_bytes(be: &dyn Backend, path: &str, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = path.rsplit_once('/') {
        be.mkdir_all(parent).map_err(|error| error.to_string())?;
    }
    let staged =
        super::unique_staging_path(be, path, "merge").map_err(|error| error.to_string())?;
    let result = (|| {
        let mut writer = be.open_write(&staged).map_err(|error| error.to_string())?;
        writer.write_all(data).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        drop(writer);
        super::promote_staged_replace(be, &staged, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = be.remove_file(&staged);
    }
    result
}

pub fn sig_from(be: &dyn Backend, path: &str) -> Result<crate::bisync::Sig, String> {
    let metadata = be.stat(path).map_err(|error| error.to_string())?;
    if metadata.is_dir || metadata.is_symlink {
        return Err(format!("Konfliktziel ist keine reguläre Datei: {path}"));
    }
    Ok(crate::bisync::Sig {
        size: metadata.size,
        mtime_ms: metadata.mtime_ms,
        hash: 0,
    })
}

pub fn rjoin(root: &str, name: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), name)
}

pub fn find_remote_unique_name(
    backend: &dyn Backend,
    parent: &str,
    candidate: impl FnMut(usize) -> String,
) -> Result<String, String> {
    find_remote_unique_name_with(|path| backend.try_exists(path), parent, candidate)
}

fn find_remote_unique_name_with(
    mut try_exists: impl FnMut(&str) -> VfsResult<bool>,
    parent: &str,
    mut candidate: impl FnMut(usize) -> String,
) -> Result<String, String> {
    for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
        let name = candidate(index);
        let path = rjoin(parent, &name);
        match try_exists(&path) {
            Ok(false) => return Ok(name),
            Ok(true) => {}
            Err(error) => return Err(format!("Ziel prüfen „{path}“: {error}")),
        }
    }
    Err(format!(
        "Kein freier Name nach {REMOTE_UNIQUE_ATTEMPTS} Versuchen"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::{Path, PathBuf};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "se_remote_util_test_{}_{}_{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn fwd(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn ensure_remote_destination_free(backend: &dyn Backend, path: &str) -> Result<(), String> {
        match backend.try_exists(path) {
            Ok(false) => Ok(()),
            Ok(true) => Err(format!("Ziel existiert bereits: {path}")),
            Err(error) => Err(format!("Ziel prüfen „{path}“: {error}")),
        }
    }

    fn numbered_name(index: usize) -> String {
        if index == 1 {
            "entry.txt".to_string()
        } else {
            format!("entry ({index}).txt")
        }
    }

    #[test]
    fn remote_unique_name_checks_the_bound_and_never_reuses_it() {
        let root = temp_dir("remote_unique_bound");
        for index in 1..1000 {
            std::fs::write(root.join(numbered_name(index)), b"occupied").unwrap();
        }
        let backend = crate::vfs::LocalBackend::new(&fwd(&root));

        let last = find_remote_unique_name(&backend, &fwd(&root), numbered_name).unwrap();
        assert_eq!(last, numbered_name(1000));
        std::fs::write(root.join(&last), b"occupied").unwrap();
        assert!(find_remote_unique_name(&backend, &fwd(&root), numbered_name).is_err());

        assert!(ensure_remote_destination_free(&backend, &fwd(&root.join(&last))).is_err());
        assert!(ensure_remote_destination_free(&backend, &fwd(&root.join("free.txt"))).is_ok());

        let probe_error = find_remote_unique_name_with(
            |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked")),
            &fwd(&root),
            numbered_name,
        )
        .expect_err("a failed existence probe must not look like a free name");
        assert!(probe_error.contains("Ziel prüfen"));
        let _ = std::fs::remove_dir_all(root);
    }
}
