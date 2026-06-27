use std::path::{Path, PathBuf};

use super::hash::verify_sha256;

#[derive(Debug)]
pub(crate) struct ReplaceTargetError {
    pub(crate) msg: String,
    pub(crate) needs_elevation: bool,
}

impl ReplaceTargetError {
    fn new(msg: impl Into<String>, needs_elevation: bool) -> Self {
        Self {
            msg: msg.into(),
            needs_elevation,
        }
    }

    fn io(context: impl Into<String>, e: std::io::Error) -> Self {
        Self::new(
            format!("{}: {}", context.into(), e),
            should_elevate_for_io(&e),
        )
    }
}

pub(crate) fn replace_target_from_staged(
    staged: &Path,
    target: &Path,
    staged_len: u64,
    expected_sha256: Option<&str>,
) -> Result<(), ReplaceTargetError> {
    let pending = unique_sibling(target, "update-pending");
    let old = unique_sibling(target, "update-old");
    let _ = std::fs::remove_file(&pending);
    copy_checked(staged, &pending, staged_len, expected_sha256)?;

    match std::fs::rename(target, &old) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::rename(&pending, target).map_err(|e| {
                let _ = std::fs::remove_file(&pending);
                ReplaceTargetError::io(format!("Einsetzen {}", target.display()), e)
            })?;
            return Ok(());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&pending);
            return Err(ReplaceTargetError::io(
                format!("Ziel sichern {}", target.display()),
                e,
            ));
        }
    }

    if let Err(e) = std::fs::rename(&pending, target) {
        let restore = std::fs::rename(&old, target);
        let _ = std::fs::remove_file(&pending);
        return match restore {
            Ok(()) => Err(ReplaceTargetError::io("Einsetzen fehlgeschlagen", e)),
            Err(restore_err) => Err(ReplaceTargetError::new(
                format!("Einsetzen fehlgeschlagen: {e}; Rollback fehlgeschlagen: {restore_err}"),
                should_elevate_for_io(&e) || should_elevate_for_io(&restore_err),
            )),
        };
    }

    let _ = std::fs::remove_file(&old);
    Ok(())
}

fn copy_checked(
    src: &Path,
    dest: &Path,
    expected_len: u64,
    expected_sha256: Option<&str>,
) -> Result<(), ReplaceTargetError> {
    let copied = match std::fs::copy(src, dest) {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            return Err(ReplaceTargetError::io(
                format!("Staging kopieren {} -> {}", src.display(), dest.display()),
                e,
            ));
        }
    };
    let actual = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if copied != expected_len || actual != expected_len {
        let _ = std::fs::remove_file(dest);
        return Err(ReplaceTargetError::new(
            format!(
                "unvollstaendig gestaged: {} von {} Bytes",
                actual.min(copied),
                expected_len
            ),
            false,
        ));
    }

    if let Some(expected_hash) = expected_sha256 {
        verify_sha256(dest, expected_hash).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            ReplaceTargetError::new(e, false)
        })?;
    }

    Ok(())
}

fn unique_sibling(target: &Path, role: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target.with_file_name(format!("{name}.{role}.{}.{}", std::process::id(), nanos))
}

fn should_elevate_for_io(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(740) | Some(1314))
        || e.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::sha256_file;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "smart-explorer-updater-replace-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn replace_rejects_same_size_tampered_staged_payload() {
        let dir = unique_temp_dir("payload");
        std::fs::create_dir_all(&dir).unwrap();
        let staged = dir.join("staged.exe");
        let target = dir.join("target.exe");
        std::fs::write(&staged, b"good").unwrap();
        std::fs::write(&target, b"old").unwrap();
        let expected = sha256_file(&staged).unwrap();
        std::fs::write(&staged, b"evil").unwrap();

        let result = replace_target_from_staged(&staged, &target, 4, Some(&expected));

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");

        let _ = std::fs::remove_dir_all(dir);
    }
}
