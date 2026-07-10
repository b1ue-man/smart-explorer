use std::path::{Path, PathBuf};

use super::hash::verify_sha256;

#[derive(Debug)]
pub(crate) struct ReplaceTargetError {
    pub(crate) msg: String,
    pub(crate) needs_elevation: bool,
}

impl ReplaceTargetError {
    pub(crate) fn new(msg: impl Into<String>, needs_elevation: bool) -> Self {
        Self {
            msg: msg.into(),
            needs_elevation,
        }
    }

    pub(crate) fn io(context: impl Into<String>, error: std::io::Error) -> Self {
        Self::new(
            format!("{}: {}", context.into(), error),
            should_elevate_for_io(&error),
        )
    }

    pub(crate) fn integrity(message: impl Into<String>) -> Self {
        Self::new(message, false)
    }
}

pub(crate) struct Replacement<'a> {
    pub(crate) label: &'a str,
    pub(crate) staged: &'a Path,
    pub(crate) target: &'a Path,
    pub(crate) sha256: &'a str,
}

struct Prepared {
    label: String,
    target: PathBuf,
    pending: PathBuf,
    old: PathBuf,
    existed: bool,
}

pub(crate) struct AppliedTransaction {
    prepared: Vec<Prepared>,
    finished: bool,
}

pub(crate) fn replace_transaction(
    replacements: &[Replacement<'_>],
) -> Result<AppliedTransaction, ReplaceTargetError> {
    let mut prepared = Vec::with_capacity(replacements.len());
    for replacement in replacements {
        if let Err(error) = verify_sha256(replacement.staged, replacement.sha256) {
            cleanup_pending(&prepared);
            return Err(ReplaceTargetError::integrity(error));
        }
        let pending = unique_sibling(replacement.target, "update-pending");
        let old = unique_sibling(replacement.target, "update-old");
        let _ = std::fs::remove_file(&pending);
        let _ = std::fs::remove_file(&old);
        if let Err(error) = copy_checked(
            replacement.staged,
            &pending,
            replacement.sha256,
            replacement.label,
        ) {
            cleanup_pending(&prepared);
            return Err(error);
        }
        prepared.push(Prepared {
            label: replacement.label.to_string(),
            target: replacement.target.to_path_buf(),
            pending,
            old,
            existed: replacement.target.exists(),
        });
    }

    for index in 0..prepared.len() {
        if !prepared[index].existed {
            continue;
        }
        if let Err(error) = std::fs::rename(&prepared[index].target, &prepared[index].old) {
            let rollback = restore_old_targets(&prepared[..index]);
            cleanup_pending(&prepared);
            return Err(combine_rollback_error(
                ReplaceTargetError::io(format!("{} Ziel sichern", prepared[index].label), error),
                rollback,
            ));
        }
    }

    for index in 0..prepared.len() {
        if let Err(error) = std::fs::rename(&prepared[index].pending, &prepared[index].target) {
            for installed in &prepared[..index] {
                let _ = std::fs::remove_file(&installed.target);
            }
            let rollback = restore_old_targets(&prepared);
            cleanup_pending(&prepared);
            return Err(combine_rollback_error(
                ReplaceTargetError::io(format!("{} einsetzen", prepared[index].label), error),
                rollback,
            ));
        }
    }

    Ok(AppliedTransaction {
        prepared,
        finished: false,
    })
}

impl AppliedTransaction {
    pub(crate) fn rollback(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        for item in &self.prepared {
            if item.existed {
                let _ = std::fs::remove_file(&item.target);
            } else if item.target.exists() {
                std::fs::remove_file(&item.target).map_err(|error| {
                    format!("Neues Ziel {} entfernen: {error}", item.target.display())
                })?;
            }
        }
        restore_old_targets(&self.prepared)?;
        cleanup_pending(&self.prepared);
        self.finished = true;
        Ok(())
    }

    pub(crate) fn finalize(&mut self) {
        if self.finished {
            return;
        }
        for item in &self.prepared {
            let _ = std::fs::remove_file(&item.old);
            let _ = std::fs::remove_file(&item.pending);
        }
        self.finished = true;
    }
}

impl Drop for AppliedTransaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback();
        }
    }
}

fn copy_checked(
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
    label: &str,
) -> Result<(), ReplaceTargetError> {
    let expected_len = std::fs::metadata(source)
        .map_err(|error| ReplaceTargetError::io(format!("{label} Quelle lesen"), error))?
        .len();
    let copied = std::fs::copy(source, destination).map_err(|error| {
        let _ = std::fs::remove_file(destination);
        ReplaceTargetError::io(format!("{label} temporaer kopieren"), error)
    })?;
    let actual_len = std::fs::metadata(destination)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if copied != expected_len || actual_len != expected_len {
        let _ = std::fs::remove_file(destination);
        return Err(ReplaceTargetError::integrity(format!(
            "{label} unvollstaendig kopiert: {} von {} Bytes",
            copied.min(actual_len),
            expected_len
        )));
    }
    verify_sha256(destination, expected_sha256).map_err(|error| {
        let _ = std::fs::remove_file(destination);
        ReplaceTargetError::integrity(error)
    })
}

fn restore_old_targets(items: &[Prepared]) -> Result<(), String> {
    let mut errors = Vec::new();
    for item in items.iter().rev().filter(|item| item.existed) {
        if item.old.exists() {
            if let Err(error) = std::fs::rename(&item.old, &item.target) {
                errors.push(format!("{}: {error}", item.target.display()));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn cleanup_pending(items: &[Prepared]) {
    for item in items {
        let _ = std::fs::remove_file(&item.pending);
    }
}

fn combine_rollback_error(
    mut primary: ReplaceTargetError,
    rollback: Result<(), String>,
) -> ReplaceTargetError {
    if let Err(error) = rollback {
        primary.msg = format!("{}; Rollback fehlgeschlagen: {error}", primary.msg);
    }
    primary
}

fn unique_sibling(target: &Path, role: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "smart_explorer".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    target.with_file_name(format!("{name}.{role}.{}.{nanos}", std::process::id()))
}

fn should_elevate_for_io(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5) | Some(740) | Some(1314))
        || error.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::sha256_file;

    #[test]
    fn transaction_rejects_tamper_without_changing_any_target() {
        let dir = tempfile::tempdir().unwrap();
        let app_staged = dir.path().join("app-staged");
        let cli_staged = dir.path().join("cli-staged");
        let app_target = dir.path().join("app");
        let cli_target = dir.path().join("cli");
        std::fs::write(&app_staged, b"new-app").unwrap();
        std::fs::write(&cli_staged, b"new-cli").unwrap();
        std::fs::write(&app_target, b"old-app").unwrap();
        std::fs::write(&cli_target, b"old-cli").unwrap();
        let app_hash = sha256_file(&app_staged).unwrap();
        let cli_hash = sha256_file(&cli_staged).unwrap();
        std::fs::write(&cli_staged, b"bad-cli").unwrap();

        let result = replace_transaction(&[
            Replacement {
                label: "App",
                staged: &app_staged,
                target: &app_target,
                sha256: &app_hash,
            },
            Replacement {
                label: "CLI",
                staged: &cli_staged,
                target: &cli_target,
                sha256: &cli_hash,
            },
        ]);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&app_target).unwrap(), b"old-app");
        assert_eq!(std::fs::read(&cli_target).unwrap(), b"old-cli");
    }

    #[test]
    fn explicit_rollback_restores_all_replaced_targets() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        let target = dir.path().join("target");
        std::fs::write(&staged, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        let hash = sha256_file(&staged).unwrap();
        let mut transaction = replace_transaction(&[Replacement {
            label: "App",
            staged: &staged,
            target: &target,
            sha256: &hash,
        }])
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");

        transaction.rollback().unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }
}
