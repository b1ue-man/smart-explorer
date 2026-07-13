use std::io::Write;

use super::{load_pending, JournalEntry, MAX_JOURNAL_BYTES};

pub(super) fn write_entry(entry: &JournalEntry) -> Result<(), String> {
    entry.validate()?;
    let encoded =
        serde_json::to_vec(entry).map_err(|error| format!("Exec-Grant journal encode: {error}"))?;
    if encoded.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("Exec-Grant journal exceeds its 64 KiB limit".into());
    }
    let destination = crate::daemon::ipc_storage::exec_journal_path()
        .map_err(|error| format!("Exec-Grant journal path: {error}"))?;
    let parent = destination
        .parent()
        .ok_or_else(|| "Exec-Grant journal has no parent directory".to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Exec-Grant journal temp: {error}"))?;
    crate::daemon::ipc_storage::secure_exec_journal_temp(temporary.as_file())
        .map_err(|error| format!("Exec-Grant journal temp security: {error}"))?;
    temporary
        .write_all(&encoded)
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| format!("Exec-Grant journal durable write: {error}"))?;
    let (file, temporary_path) = temporary
        .keep()
        .map_err(|error| format!("Exec-Grant journal retain temp: {}", error.error))?;
    drop(file);
    crate::daemon::ipc_storage::commit_exec_journal_temp(&temporary_path, &destination)
        .map_err(|error| format!("Exec-Grant journal commit: {error}"))?;
    match load_pending()? {
        Some(stored) if stored == *entry => Ok(()),
        Some(_) => Err("Exec-Grant journal verification mismatch".into()),
        None => Err("Exec-Grant journal disappeared after commit".into()),
    }
}

pub(super) fn clear_entry(operation_id: &str) -> Result<(), String> {
    let stored = load_pending()?
        .ok_or_else(|| "Exec-Grant journal disappeared before acknowledgement".to_string())?;
    if stored.operation_id != operation_id {
        return Err("Exec-Grant journal operation changed before acknowledgement".into());
    }
    let path = crate::daemon::ipc_storage::exec_journal_path()
        .map_err(|error| format!("Exec-Grant journal path: {error}"))?;
    unlink_and_sync_with_recovery(
        &stored,
        || {
            std::fs::remove_file(&path)
                .map_err(|error| format!("Exec-Grant journal remove: {error}"))
        },
        || {
            crate::daemon::ipc_storage::sync_exec_journal_directory()
                .map_err(|error| format!("Exec-Grant journal directory sync: {error}"))
        },
        write_entry,
        || match load_pending()? {
            Some(recovered) if recovered == stored => Ok(true),
            Some(_) => Err("restored Exec-Grant journal verification mismatch".into()),
            None => Ok(false),
        },
    )
}

pub(super) fn unlink_and_sync_with_recovery<T, R, S, W, V>(
    stored: &T,
    mut remove: R,
    mut sync_directory: S,
    mut restore: W,
    mut verify_restored: V,
) -> Result<(), String>
where
    R: FnMut() -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
    W: FnMut(&T) -> Result<(), String>,
    V: FnMut() -> Result<bool, String>,
{
    remove()?;
    let sync_error = match sync_directory() {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    let restore_error = restore(stored).err();
    let verification = verify_restored();
    let recovery = match (restore_error, verification) {
        (None, Ok(true)) => "recovery journal restored and verified".to_string(),
        (Some(error), Ok(true)) => format!(
            "recovery journal is present and verified, but its durable write reported: {error}"
        ),
        (None, Ok(false)) => "recovery journal restoration was not visible".to_string(),
        (Some(error), Ok(false)) => {
            format!("recovery journal restoration failed: {error}")
        }
        (None, Err(error)) => format!("recovery journal verification failed: {error}"),
        (Some(restore), Err(verify)) => {
            format!("recovery journal restoration failed: {restore}; verification failed: {verify}")
        }
    };
    Err(format!("{sync_error}; {recovery}"))
}
