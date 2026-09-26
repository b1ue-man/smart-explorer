//! In-place replacement of a terminal-only `se`: one update at a time per
//! installed file, a hash-verified backup, an atomic swap, and an undo that
//! also runs when the user interrupts `se update` with Ctrl+C.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::core::{copy_file_checked, sha256_file, unique_sibling, verify_sha256};
use super::os;

/// What an interrupt must undo while the installed file is being swapped.
struct InterruptUndo {
    target: PathBuf,
    backup: PathBuf,
    backup_sha256: String,
    pending: PathBuf,
    lock: PathBuf,
    swapped: bool,
}

static INTERRUPT_UNDO: Mutex<Option<InterruptUndo>> = Mutex::new(None);

fn undo_slot() -> Result<MutexGuard<'static, Option<InterruptUndo>>, String> {
    INTERRUPT_UNDO
        .lock()
        .map_err(|_| "the update's interrupt state is poisoned".to_string())
}

/// Ctrl+C during the swap puts the previous file back (or removes the
/// unfinished copies) before the process ends with status 130.
pub(super) fn arm_interrupt_undo() -> Result<(), String> {
    ctrlc::set_handler(|| {
        undo_after_interrupt();
        std::process::exit(130);
    })
    .map_err(|error| format!("Ctrl+C handler: {error}"))
}

fn undo_after_interrupt() {
    let Ok(mut slot) = INTERRUPT_UNDO.lock() else {
        return;
    };
    let Some(undo) = slot.take() else {
        return;
    };
    if undo.swapped {
        if sha256_matches(&undo.backup, &undo.backup_sha256) {
            let _ = std::fs::rename(&undo.backup, &undo.target);
        }
    } else {
        let _ = std::fs::remove_file(&undo.pending);
        let _ = std::fs::remove_file(&undo.backup);
    }
    let _ = std::fs::remove_file(&undo.lock);
}

fn sha256_matches(path: &Path, expected: &str) -> bool {
    sha256_file(path).is_ok_and(|actual| actual.eq_ignore_ascii_case(expected))
}

/// `<name>.update-lock` beside the installed file, holding the owner's PID.
pub(super) struct UpdateLock {
    path: PathBuf,
}

impl UpdateLock {
    pub(super) fn acquire(target: &Path) -> Result<Self, String> {
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no file name", target.display()))?;
        let path = target.with_file_name(format!("{name}.update-lock"));
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    write!(file, "{}", std::process::id())
                        .map_err(|error| format!("write {}: {error}", path.display()))?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let owner = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|text| text.trim().parse::<u32>().ok());
                    match owner {
                        Some(pid) if os::process_alive(pid) => {
                            let target = target.display();
                            return Err(format!(
                                "another se update (process {pid}) is replacing {target}"
                            ));
                        }
                        Some(_) => {
                            let _ = std::fs::remove_file(&path);
                        }
                        None => {
                            let lock = path.display();
                            return Err(format!(
                                "{lock} names no update process; remove it if no se update is running"
                            ));
                        }
                    }
                }
                Err(error) => return Err(format!("lock {}: {error}", path.display())),
            }
        }
        Err(format!("could not lock {}", path.display()))
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Removes `<name>.update-old.<pid>.<n>` and `<name>.update-pending.<pid>.<n>`
/// left by an update process that no longer runs. The running `se` is the
/// installed file, so a leftover backup is never the only working copy.
pub(super) fn remove_orphans(target: &Path) {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return;
    };
    let name = name.to_string_lossy();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let candidate = entry.file_name();
        let Some(pid) = orphan_owner(&name, &candidate.to_string_lossy()) else {
            continue;
        };
        if pid != std::process::id() && !os::process_alive(pid) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn orphan_owner(name: &str, candidate: &str) -> Option<u32> {
    let rest = candidate.strip_prefix(name)?;
    let rest = rest
        .strip_prefix(".update-old.")
        .or_else(|| rest.strip_prefix(".update-pending."))?;
    let (pid, nanos) = rest.split_once('.')?;
    nanos.parse::<u128>().ok()?;
    pid.parse().ok()
}

/// A replaced terminal-only `se`. The hash-verified previous file stays
/// beside it until the new one has proven that it starts.
#[must_use = "commit or roll back the replaced se"]
pub struct ReplacedCli {
    target: PathBuf,
    backup: PathBuf,
    backup_sha256: String,
    sha256: String,
    _lock: UpdateLock,
}

impl ReplacedCli {
    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn backup(&self) -> &Path {
        &self.backup
    }

    /// Compares the installed file with the verified payload right before it
    /// is started; nothing is deleted on a mismatch.
    pub fn verify_installed(&self) -> Result<(), String> {
        let actual = sha256_file(&self.target)?;
        if actual.eq_ignore_ascii_case(&self.sha256) {
            Ok(())
        } else {
            let (target, expected) = (self.target.display(), &self.sha256);
            Err(format!(
                "{target} changed after the update (SHA-256 {actual}, expected {expected})"
            ))
        }
    }

    pub fn commit(self) -> Result<(), String> {
        undo_slot()?.take();
        std::fs::remove_file(&self.backup)
            .map_err(|error| format!("remove the backup {}: {error}", self.backup.display()))
    }

    /// Puts the previous file back with one atomic rename after checking that
    /// the backup still holds exactly the bytes that were installed.
    pub fn rollback(self) -> Result<(), String> {
        let mut slot = undo_slot()?;
        let backup = self.backup.display();
        if !sha256_matches(&self.backup, &self.backup_sha256) {
            return Err(format!(
                "the backup {backup} no longer matches the replaced se and was left in place"
            ));
        }
        std::fs::rename(&self.backup, &self.target)
            .map_err(|error| format!("restore the previous se from {backup}: {error}"))?;
        slot.take();
        Ok(())
    }
}

/// Copies the verified payload beside `target` with the installed file's
/// permissions, verifies that copy again and renames it over `target`, so the
/// name never disappears and never refers to unverified bytes.
pub(super) fn install_cli_in_place(
    staged: &Path,
    sha256: &str,
    target: &Path,
    lock: UpdateLock,
) -> Result<ReplacedCli, String> {
    verify_sha256(staged, sha256)?;
    let permissions = std::fs::metadata(target)
        .map_err(|error| format!("read the installed se {}: {error}", target.display()))?
        .permissions();
    let backup_sha256 = sha256_file(target)?;
    let backup = unique_sibling(target, "update-old");
    let pending = unique_sibling(target, "update-pending");
    *undo_slot()? = Some(InterruptUndo {
        target: target.to_path_buf(),
        backup: backup.clone(),
        backup_sha256: backup_sha256.clone(),
        pending: pending.clone(),
        lock: lock.path.clone(),
        swapped: false,
    });
    let backup_hash = Some(backup_sha256.as_str());
    let prepared = copy_file_checked(target, &backup, "backup of se", backup_hash)
        .and_then(|()| copy_file_checked(staged, &pending, "new se", Some(sha256)))
        .and_then(|()| {
            std::fs::set_permissions(&pending, permissions)
                .map_err(|error| format!("set the permissions of the new se: {error}"))
        })
        .and_then(|()| verify_sha256(&pending, sha256));
    let swapped = prepared.and_then(|()| {
        // Swap and record it under the undo lock, so an interrupt sees either
        // state completely.
        let mut slot = undo_slot()?;
        std::fs::rename(&pending, target)
            .map_err(|error| format!("install the new se ({}): {error}", target.display()))?;
        if let Some(undo) = slot.as_mut() {
            undo.swapped = true;
        }
        Ok(())
    });
    if let Err(error) = swapped {
        if let Ok(mut slot) = undo_slot() {
            slot.take();
        }
        let _ = std::fs::remove_file(&pending);
        let _ = std::fs::remove_file(&backup);
        return Err(error);
    }
    Ok(ReplacedCli {
        target: target.to_path_buf(),
        backup,
        backup_sha256,
        sha256: sha256.to_string(),
        _lock: lock,
    })
}

#[cfg(test)]
mod tests {
    use super::{install_cli_in_place, orphan_owner, UpdateLock};

    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn file_sha256(path: &std::path::Path) -> String {
        super::super::core::sha256_file(path).unwrap()
    }

    fn setup(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf, String) {
        let target = dir.join("se");
        std::fs::write(&target, b"old se").unwrap();
        let staged = dir.join("staged");
        std::fs::write(&staged, b"new se").unwrap();
        let sha256 = file_sha256(&staged);
        (target, staged, sha256)
    }

    #[test]
    fn cli_task_install_replaces_in_place_commits_and_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let (target, staged, sha256) = setup(dir.path());

        let lock = UpdateLock::acquire(&target).unwrap();
        let replaced = install_cli_in_place(&staged, &sha256, &target, lock).unwrap();
        assert_eq!(replaced.target(), target.as_path());
        assert_eq!(replaced.sha256(), sha256);
        assert_eq!(std::fs::read(&target).unwrap(), b"new se");
        replaced.verify_installed().unwrap();
        replaced.rollback().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old se");
        assert_eq!(names(dir.path()), ["se", "staged"]);

        let lock = UpdateLock::acquire(&target).unwrap();
        let replaced = install_cli_in_place(&staged, &sha256, &target, lock).unwrap();
        replaced.commit().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new se");
        assert_eq!(names(dir.path()), ["se", "staged"]);
    }

    #[test]
    fn cli_task_install_rejects_a_hash_mismatch_before_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let (target, staged, _) = setup(dir.path());
        let lock = UpdateLock::acquire(&target).unwrap();
        let expected = "0".repeat(64);
        assert!(install_cli_in_place(&staged, &expected, &target, lock).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old se");
        // The mismatching download is discarded and nothing else is left behind.
        assert_eq!(names(dir.path()), ["se"]);
    }

    #[test]
    fn cli_task_install_checks_the_backup_and_the_installed_file() {
        let dir = tempfile::tempdir().unwrap();
        let (target, staged, sha256) = setup(dir.path());
        let lock = UpdateLock::acquire(&target).unwrap();
        let replaced = install_cli_in_place(&staged, &sha256, &target, lock).unwrap();
        let backup = replaced.backup().to_path_buf();

        std::fs::write(&target, b"changed").unwrap();
        assert!(replaced
            .verify_installed()
            .unwrap_err()
            .contains("changed after the update"));

        std::fs::write(&backup, b"tampered backup").unwrap();
        let error = replaced.rollback().unwrap_err();
        assert!(error.contains("no longer matches"));
        assert!(error.contains(&backup.display().to_string()));
        assert_eq!(std::fs::read(&backup).unwrap(), b"tampered backup");
    }

    #[test]
    fn cli_task_update_lock_is_exclusive_and_names_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("se");
        let lock = UpdateLock::acquire(&target).unwrap();
        assert!(UpdateLock::acquire(&target)
            .err()
            .unwrap()
            .contains("another se update"));
        drop(lock);
        drop(UpdateLock::acquire(&target).unwrap());

        assert_eq!(orphan_owner("se", "se.update-old.4242.17"), Some(4242));
        assert_eq!(orphan_owner("se", "se.update-pending.7.123"), Some(7));
        assert_eq!(orphan_owner("se", "se.update-lock"), None);
        assert_eq!(orphan_owner("se", "se.update-old.x.1"), None);
        assert_eq!(orphan_owner("se", "sed.update-old.1.1"), None);
    }
}
