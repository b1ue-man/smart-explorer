use std::path::{Path, PathBuf};

use super::hash::{sha256_file, verify_sha256};

#[path = "replace_platform.rs"]
mod replace_platform;
pub(crate) use replace_platform::ReplaceTargetError;
use replace_platform::{copy_checked, ensure_missing, verify_regular_sha256};

pub(crate) struct Replacement<'a> {
    pub(crate) label: &'a str,
    pub(crate) staged: &'a Path,
    pub(crate) target: &'a Path,
    pub(crate) sha256: &'a str,
    pub(crate) expected_target_sha256: Option<&'a str>,
}

struct Prepared {
    label: String,
    target: PathBuf,
    pending: PathBuf,
    old: PathBuf,
    existed: bool,
    original_sha256: Option<String>,
    new_sha256: String,
    post_install_invalid: bool,
}

pub(crate) struct AppliedTransaction {
    prepared: Vec<Prepared>,
    finished: bool,
}

pub(crate) fn replace_transaction(
    replacements: &[Replacement<'_>],
) -> Result<AppliedTransaction, ReplaceTargetError> {
    replace_transaction_impl(replacements, || {})
}

fn replace_transaction_impl(
    replacements: &[Replacement<'_>],
    before_first_install: impl FnOnce(),
) -> Result<AppliedTransaction, ReplaceTargetError> {
    let mut prepared = Vec::with_capacity(replacements.len());
    for replacement in replacements {
        if let Err(error) = verify_regular_sha256(
            replacement.staged,
            replacement.sha256,
            &format!("{} Quelle", replacement.label),
        ) {
            return Err(with_cleanup_warnings(error, cleanup_pending(&prepared)));
        }
        let pending = replace_platform::unique_sibling(replacement.target, "update-pending");
        let old = replace_platform::unique_sibling(replacement.target, "update-old");
        let (existed, original_sha256) = match original_target(replacement) {
            Ok(value) => value,
            Err(error) => {
                return Err(with_cleanup_warnings(error, cleanup_pending(&prepared)));
            }
        };
        if let Err(error) = copy_checked(
            replacement.staged,
            &pending,
            replacement.sha256,
            replacement.label,
        ) {
            return Err(with_cleanup_warnings(error, cleanup_pending(&prepared)));
        }
        prepared.push(Prepared {
            label: replacement.label.to_string(),
            target: replacement.target.to_path_buf(),
            pending,
            old,
            existed,
            original_sha256,
            new_sha256: replacement.sha256.to_string(),
            post_install_invalid: false,
        });
    }

    before_first_install();
    for index in 0..prepared.len() {
        if let Err(error) = install_one(&mut prepared[index]) {
            let rollback = rollback_items(&prepared[..=index]);
            let mut error = combine_rollback_error(error, rollback);
            error = with_cleanup_warnings(error, cleanup_pending(&prepared));
            return Err(error);
        }
    }

    Ok(AppliedTransaction {
        prepared,
        finished: false,
    })
}

fn original_target(
    replacement: &Replacement<'_>,
) -> Result<(bool, Option<String>), ReplaceTargetError> {
    match std::fs::symlink_metadata(replacement.target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let hash = sha256_file(replacement.target).map_err(ReplaceTargetError::integrity)?;
            if let Some(expected) = replacement.expected_target_sha256 {
                if !hash.eq_ignore_ascii_case(expected) {
                    return Err(ReplaceTargetError::integrity(format!(
                        "{} Ziel hat eine unerwartete Pruefsumme: erwartet {expected}, erhalten {hash}",
                        replacement.label
                    )));
                }
            }
            Ok((true, Some(hash)))
        }
        Ok(_) => Err(ReplaceTargetError::integrity(format!(
            "{} Ziel {} ist keine regulaere Datei",
            replacement.label,
            replacement.target.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if replacement.expected_target_sha256.is_some() {
                Err(ReplaceTargetError::integrity(format!(
                    "{} erwartetes Ziel {} fehlt",
                    replacement.label,
                    replacement.target.display()
                )))
            } else {
                Ok((false, None))
            }
        }
        Err(error) => Err(ReplaceTargetError::io(
            format!("{} Ziel pruefen", replacement.label),
            error,
        )),
    }
}

fn install_one(item: &mut Prepared) -> Result<(), ReplaceTargetError> {
    if item.existed {
        install_over_existing(item)
    } else {
        verify_regular_sha256(
            &item.pending,
            &item.new_sha256,
            &format!("{} vorbereitete Datei", item.label),
        )?;
        ensure_missing(&item.target, &format!("{} Ziel", item.label))?;
        replace_platform::rename_no_replace(&item.pending, &item.target).map_err(|error| {
            ReplaceTargetError::io(format!("{} neues Ziel einsetzen", item.label), error)
        })?;
        verify_regular_sha256(
            &item.target,
            &item.new_sha256,
            &format!("{} eingesetzte Datei", item.label),
        )
    }
}

fn install_over_existing(item: &mut Prepared) -> Result<(), ReplaceTargetError> {
    let original = item.original_sha256.clone().ok_or_else(|| {
        ReplaceTargetError::integrity(format!("{} urspruengliche Pruefsumme fehlt", item.label))
    })?;
    let result = replace_platform::replace_existing_with_guard(
        &item.pending,
        &item.target,
        &item.old,
        |backup_ready| {
            verify_regular_sha256(
                &item.pending,
                &item.new_sha256,
                &format!("{} vorbereitete Datei", item.label),
            )?;
            verify_regular_sha256(
                &item.target,
                &original,
                &format!("{} Ziel unmittelbar vor dem Ersetzen", item.label),
            )?;
            if backup_ready {
                verify_regular_sha256(
                    &item.old,
                    &original,
                    &format!("{} Rollback-Sicherung", item.label),
                )?;
            }
            Ok(())
        },
    );
    match result {
        Ok(()) => {}
        Err(replace_platform::InstallError::Guard(error)) => return Err(error),
        Err(replace_platform::InstallError::Io(error)) => {
            let mut failure =
                ReplaceTargetError::io(format!("{} atomar einsetzen", item.label), error);
            if let Err(recovery) = replace_platform::recover_failed_install(item) {
                failure.msg = format!(
                    "{}; unmittelbare Wiederherstellung: {recovery}",
                    failure.msg
                );
            }
            return Err(failure);
        }
    }
    item.post_install_invalid = true;
    verify_regular_sha256(
        &item.old,
        &original,
        &format!("{} Rollback-Sicherung nach dem Ersetzen", item.label),
    )?;
    verify_regular_sha256(
        &item.target,
        &item.new_sha256,
        &format!("{} eingesetzte Datei", item.label),
    )?;
    item.post_install_invalid = false;
    Ok(())
}

pub(crate) fn replace_transaction_with_retries(
    replacements: &[Replacement<'_>],
) -> Result<AppliedTransaction, ReplaceTargetError> {
    let mut last = None;
    for _ in 0..10 {
        match replace_transaction(replacements) {
            Ok(transaction) => return Ok(transaction),
            Err(error)
                if error.needs_elevation
                    || error.msg.contains("Rollback fehlgeschlagen")
                    || error.msg.contains("Pruefsumme") =>
            {
                return Err(error);
            }
            Err(error) => last = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(last.unwrap_or_else(|| ReplaceTargetError::new("unbekannter Fehler", false)))
}

impl AppliedTransaction {
    pub(crate) fn rollback(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        rollback_items(&self.prepared)?;
        let warnings = cleanup_pending(&self.prepared);
        if !warnings.is_empty() {
            return Err(warnings.join("; "));
        }
        self.finished = true;
        Ok(())
    }

    pub(crate) fn finalize(&mut self) {
        for warning in self.finish_cleanup() {
            super::logging::append_log(&format!("warning: {warning}"));
        }
    }

    fn finish_cleanup(&mut self) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for item in &self.prepared {
            remove_artifact(&item.old, "Rollback-Sicherung", &mut warnings);
            remove_artifact(&item.pending, "vorbereitete Datei", &mut warnings);
        }
        self.finished = true;
        warnings
    }
}

impl Drop for AppliedTransaction {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.rollback() {
                super::logging::append_log(&format!(
                    "warning: automatischer Update-Rollback fehlgeschlagen: {error}"
                ));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RollbackAction {
    Restore,
    RestoreInvalid,
    AlreadyOriginal,
    RemoveNew,
    Untouched,
}

fn rollback_items(items: &[Prepared]) -> Result<(), String> {
    let actions = items
        .iter()
        .map(rollback_action)
        .collect::<Result<Vec<_>, _>>()?;
    let mut errors = Vec::new();
    for (item, action) in items.iter().zip(actions).rev() {
        let result = match action {
            RollbackAction::Restore => restore_one(item),
            RollbackAction::RestoreInvalid => restore_invalid(item),
            RollbackAction::AlreadyOriginal => remove_verified_backup(item),
            RollbackAction::RemoveNew => remove_new_target(item),
            RollbackAction::Untouched => Ok(()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn rollback_action(item: &Prepared) -> Result<RollbackAction, String> {
    let target_hash = replace_platform::regular_file_hash(&item.target, "Rollback-Ziel")?;
    if item.existed {
        let original = item
            .original_sha256
            .as_deref()
            .ok_or_else(|| format!("Rollback-Pruefsumme fuer {} fehlt", item.target.display()))?;
        match target_hash.as_deref() {
            Some(hash) if hash.eq_ignore_ascii_case(original) => {
                if item.old.exists() {
                    replace_platform::verify_file_hash(&item.old, original, "Rollback-Sicherung")?;
                }
                Ok(RollbackAction::AlreadyOriginal)
            }
            Some(hash) if hash.eq_ignore_ascii_case(&item.new_sha256) => {
                replace_platform::verify_file_hash(&item.old, original, "Rollback-Sicherung")?;
                Ok(RollbackAction::Restore)
            }
            _ if item.post_install_invalid => {
                replace_platform::verify_file_hash(&item.old, original, "Rollback-Sicherung")?;
                Ok(RollbackAction::RestoreInvalid)
            }
            Some(hash) => Err(format!(
                "Rollback-Ziel {} wurde unerwartet veraendert (Pruefsumme {hash})",
                item.target.display()
            )),
            None => Err(format!("Rollback-Ziel {} fehlt", item.target.display())),
        }
    } else {
        match target_hash.as_deref() {
            None => Ok(RollbackAction::Untouched),
            Some(hash) if hash.eq_ignore_ascii_case(&item.new_sha256) => {
                Ok(RollbackAction::RemoveNew)
            }
            Some(hash) => Err(format!(
                "Neues Rollback-Ziel {} wurde unerwartet veraendert (Pruefsumme {hash})",
                item.target.display()
            )),
        }
    }
}

fn restore_one(item: &Prepared) -> Result<(), String> {
    let original = item
        .original_sha256
        .as_deref()
        .ok_or_else(|| format!("Rollback-Pruefsumme fuer {} fehlt", item.target.display()))?;
    replace_platform::restore_original(&item.old, &item.target, original, Some(&item.new_sha256))
}

fn restore_invalid(item: &Prepared) -> Result<(), String> {
    let original = item
        .original_sha256
        .as_deref()
        .ok_or_else(|| format!("Rollback-Pruefsumme fuer {} fehlt", item.target.display()))?;
    replace_platform::restore_original(&item.old, &item.target, original, None)
}

fn remove_verified_backup(item: &Prepared) -> Result<(), String> {
    let Some(original) = item.original_sha256.as_deref() else {
        return Err(format!(
            "Rollback-Pruefsumme fuer {} fehlt",
            item.target.display()
        ));
    };
    replace_platform::verify_file_hash(&item.target, original, "bereits wiederhergestelltes Ziel")?;
    if item.old.exists() {
        replace_platform::verify_file_hash(&item.old, original, "Rollback-Sicherung")?;
        replace_platform::remove_file(&item.old)
            .map_err(|error| format!("{} entfernen: {error}", item.old.display()))?;
    }
    Ok(())
}

fn remove_new_target(item: &Prepared) -> Result<(), String> {
    replace_platform::verify_file_hash(&item.target, &item.new_sha256, "neues Rollback-Ziel")?;
    replace_platform::remove_file(&item.target)
        .map_err(|error| format!("Neues Ziel {} entfernen: {error}", item.target.display()))?;
    match std::fs::symlink_metadata(&item.target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "Neues Ziel {} besteht weiter",
            item.target.display()
        )),
        Err(error) => Err(format!(
            "Neues Ziel {} pruefen: {error}",
            item.target.display()
        )),
    }
}

fn cleanup_pending(items: &[Prepared]) -> Vec<String> {
    let mut warnings = Vec::new();
    for item in items {
        remove_artifact(&item.pending, "vorbereitete Datei", &mut warnings);
    }
    warnings
}

fn remove_artifact(path: &Path, label: &str, warnings: &mut Vec<String>) {
    if let Err(error) = replace_platform::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warnings.push(format!("{label} {} entfernen: {error}", path.display()));
        }
    }
}

fn with_cleanup_warnings(
    mut error: ReplaceTargetError,
    warnings: Vec<String>,
) -> ReplaceTargetError {
    if !warnings.is_empty() {
        error.msg = format!(
            "{}; Aufraeumen fehlgeschlagen: {}",
            error.msg,
            warnings.join("; ")
        );
    }
    error
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

#[cfg(test)]
#[path = "replace_tests.rs"]
mod tests;
