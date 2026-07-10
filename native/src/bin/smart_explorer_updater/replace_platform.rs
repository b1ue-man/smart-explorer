use std::io;
use std::path::{Path, PathBuf};

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

    pub(crate) fn io(context: impl Into<String>, error: io::Error) -> Self {
        let needs_elevation = matches!(error.raw_os_error(), Some(5) | Some(740) | Some(1314))
            || error.kind() == io::ErrorKind::PermissionDenied;
        Self::new(format!("{}: {}", context.into(), error), needs_elevation)
    }

    pub(crate) fn integrity(message: impl Into<String>) -> Self {
        Self::new(message, false)
    }
}

#[derive(Debug)]
pub(super) enum InstallError<E> {
    Guard(E),
    Io(io::Error),
}

#[cfg(windows)]
pub(super) fn replace_existing_with_guard<E>(
    pending: &Path,
    target: &Path,
    backup: &Path,
    guard: impl FnOnce(bool) -> Result<(), E>,
) -> Result<(), InstallError<E>> {
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    // ReplaceFileW swaps the prepared sibling and creates its backup atomically.
    guard(false).map_err(InstallError::Guard)?;
    let target = wide(target);
    let pending = wide(pending);
    let backup = wide(backup);
    let result = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            pending.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        // Errors 1175-1177 can leave partial names; the caller hash-classifies all three.
        Err(InstallError::Io(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn replace_existing_with_guard<E>(
    pending: &Path,
    target: &Path,
    backup: &Path,
    guard: impl FnOnce(bool) -> Result<(), E>,
) -> Result<(), InstallError<E>> {
    create_backup(target, backup).map_err(InstallError::Io)?;
    sync_parent(backup).map_err(InstallError::Io)?;
    guard(true).map_err(InstallError::Guard)?;
    std::fs::rename(pending, target).map_err(InstallError::Io)?;
    sync_parent(target).map_err(InstallError::Io)
}

#[cfg(target_os = "linux")]
fn create_backup(target: &Path, backup: &Path) -> io::Result<()> {
    match std::fs::hard_link(target, backup) {
        Ok(()) => Ok(()),
        Err(error) if hard_link_can_fall_back(&error) => copy_backup(target, backup),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn hard_link_can_fall_back(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || error.kind() == io::ErrorKind::Unsupported
        || matches!(
            error.raw_os_error(),
            Some(libc::EPERM) | Some(libc::EOPNOTSUPP)
        )
}

#[cfg(target_os = "linux")]
fn copy_backup(target: &Path, backup: &Path) -> io::Result<()> {
    use std::io::Write;

    let mut created = false;
    let result = (|| {
        let mut source = std::fs::File::open(target)?;
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(backup)?;
        created = true;
        std::io::copy(&mut source, &mut destination)?;
        destination.set_permissions(source.metadata()?.permissions())?;
        destination.flush()?;
        destination.sync_all()
    })();
    if result.is_err() && created {
        let _ = remove_file(backup);
    }
    result
}

#[cfg(windows)]
pub(super) fn rename_no_replace(pending: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let pending: Vec<u16> = pending.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe { MoveFileExW(pending.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn rename_no_replace(pending: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let pending = CString::new(pending.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "pending path contains NUL"))?;
    let target_path = target;
    let target = CString::new(target_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            pending.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        sync_parent(target_path)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(super) fn restore_existing(backup: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let target = wide(target);
    let backup = wide(backup);
    let result = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            backup.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn restore_existing(backup: &Path, target: &Path) -> io::Result<()> {
    std::fs::rename(backup, target)?;
    sync_parent(target)
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn replace_existing_with_guard<E>(
    _pending: &Path,
    _target: &Path,
    _backup: &Path,
    _guard: impl FnOnce(bool) -> Result<(), E>,
) -> Result<(), InstallError<E>> {
    Err(InstallError::Io(unsupported()))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn rename_no_replace(_pending: &Path, _target: &Path) -> io::Result<()> {
    Err(unsupported())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn restore_existing(_backup: &Path, _target: &Path) -> io::Result<()> {
    Err(unsupported())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic updater replacement is unsupported on this operating system",
    )
}

pub(super) fn regular_file_hash(path: &Path, label: &str) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => super::sha256_file(path).map(Some),
        Ok(_) => Err(format!(
            "{label} {} ist keine regulaere Datei",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{label} {} pruefen: {error}", path.display())),
    }
}

pub(super) fn verify_file_hash(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let hash = regular_file_hash(path, label)?
        .ok_or_else(|| format!("{label} {} fehlt", path.display()))?;
    if hash.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "{label} {} hat Pruefsumme {hash}, erwartet {expected}",
            path.display()
        ))
    }
}

pub(super) fn unique_sibling(target: &Path, role: &str) -> PathBuf {
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

#[cfg(target_os = "linux")]
pub(super) fn sync_parent(path: &Path) -> io::Result<()> {
    std::fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)?;
    sync_parent(path)
}

pub(super) fn copy_checked(
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
    label: &str,
) -> Result<(), ReplaceTargetError> {
    use std::io::Write;

    let mut created = false;
    let result = (|| {
        let mut source_file = std::fs::File::open(source)
            .map_err(|error| ReplaceTargetError::io(format!("{label} Quelle lesen"), error))?;
        let source_metadata = source_file
            .metadata()
            .map_err(|error| ReplaceTargetError::io(format!("{label} Quelle lesen"), error))?;
        let mut destination_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| ReplaceTargetError::io(format!("{label} temporaer anlegen"), error))?;
        created = true;
        let copied = std::io::copy(&mut source_file, &mut destination_file).map_err(|error| {
            ReplaceTargetError::io(format!("{label} temporaer kopieren"), error)
        })?;
        if copied != source_metadata.len() {
            return Err(ReplaceTargetError::integrity(format!(
                "{label} unvollstaendig kopiert: {copied} von {} Bytes",
                source_metadata.len()
            )));
        }
        destination_file
            .set_permissions(source_metadata.permissions())
            .and_then(|_| destination_file.flush())
            .and_then(|_| destination_file.sync_all())
            .map_err(|error| ReplaceTargetError::io(format!("{label} temporaer sichern"), error))?;
        sync_parent(destination).map_err(|error| {
            ReplaceTargetError::io(format!("{label} temporaeren Eintrag sichern"), error)
        })?;
        verify_regular_sha256(
            destination,
            expected_sha256,
            &format!("{label} temporaere Datei"),
        )
    })();
    if result.is_err() && created {
        let _ = remove_file(destination);
    }
    result
}

pub(super) fn verify_regular_sha256(
    path: &Path,
    expected: &str,
    label: &str,
) -> Result<(), ReplaceTargetError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(ReplaceTargetError::integrity(format!(
                "{label} {} ist keine regulaere Datei",
                path.display()
            )));
        }
        Err(error) => {
            return Err(ReplaceTargetError::io(
                format!("{label} {} pruefen", path.display()),
                error,
            ));
        }
    }
    super::verify_sha256(path, expected).map_err(|error| {
        ReplaceTargetError::integrity(format!("{label} {}: {error}", path.display()))
    })
}

pub(super) fn ensure_missing(path: &Path, label: &str) -> Result<(), ReplaceTargetError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(ReplaceTargetError::integrity(format!(
            "{label} {} ist unerwartet vorhanden",
            path.display()
        ))),
        Err(error) => Err(ReplaceTargetError::io(
            format!("{label} {} pruefen", path.display()),
            error,
        )),
    }
}

pub(super) fn recover_failed_install(item: &super::Prepared) -> Result<(), String> {
    let original = item
        .original_sha256
        .as_deref()
        .ok_or_else(|| format!("Rollback-Pruefsumme fuer {} fehlt", item.target.display()))?;
    let target_hash = regular_file_hash(&item.target, "Ziel nach fehlgeschlagenem Ersetzen")?;
    let backup_hash = regular_file_hash(&item.old, "Rollback-Sicherung");
    match target_hash.as_deref() {
        Some(hash) if hash.eq_ignore_ascii_case(original) => validate_leftovers(item, original),
        Some(hash) if hash.eq_ignore_ascii_case(&item.new_sha256) => {
            restore_original(&item.old, &item.target, original, Some(&item.new_sha256))?;
            validate_leftovers(item, original)
        }
        Some(hash) => Err(format!(
            "Ziel {} hat unerwartete Pruefsumme {hash}",
            item.target.display()
        )),
        None if backup_hash.as_ref().is_ok_and(|hash| {
            hash.as_deref()
                .is_some_and(|hash| hash.eq_ignore_ascii_case(original))
        }) =>
        {
            restore_original(&item.old, &item.target, original, None)?;
            validate_leftovers(item, original)
        }
        None => {
            let pending_hash = regular_file_hash(&item.pending, "vorbereitete Datei")?;
            require_known_hash(pending_hash.as_deref(), &item.new_sha256, &item.pending)?;
            if pending_hash.is_none() {
                return Err(backup_hash
                    .err()
                    .unwrap_or_else(|| "keine verifizierte Wiederherstellungsdatei".to_string()));
            }
            rename_no_replace(&item.pending, &item.target)
                .map_err(|error| format!("verifiziertes neues Ziel wieder einsetzen: {error}"))?;
            verify_file_hash(
                &item.target,
                &item.new_sha256,
                "wieder eingesetztes neues Ziel",
            )?;
            Err(format!(
                "{}; nur die neue Version konnte lauffaehig gehalten werden",
                backup_hash
                    .err()
                    .unwrap_or_else(|| "vorherige Version fehlt".to_string())
            ))
        }
    }
}

fn validate_leftovers(item: &super::Prepared, original: &str) -> Result<(), String> {
    let pending = regular_file_hash(&item.pending, "vorbereitete Datei")?;
    let backup = regular_file_hash(&item.old, "Rollback-Sicherung")?;
    require_known_hash(pending.as_deref(), &item.new_sha256, &item.pending)?;
    require_known_hash(backup.as_deref(), original, &item.old)
}

fn require_known_hash(actual: Option<&str>, expected: &str, path: &Path) -> Result<(), String> {
    if actual.is_none_or(|hash| hash.eq_ignore_ascii_case(expected)) {
        Ok(())
    } else {
        Err(format!(
            "{} hat eine unerwartete Pruefsumme",
            path.display()
        ))
    }
}

pub(super) fn restore_original(
    backup: &Path,
    target: &Path,
    original_sha256: &str,
    expected_current_sha256: Option<&str>,
) -> Result<(), String> {
    verify_file_hash(backup, original_sha256, "Rollback-Sicherung")?;
    match regular_file_hash(target, "Rollback-Ziel")? {
        None => {
            rename_no_replace(backup, target)
                .map_err(|error| format!("fehlendes Rollback-Ziel wiederherstellen: {error}"))?;
            return verify_file_hash(target, original_sha256, "wiederhergestelltes Ziel");
        }
        Some(hash) => {
            if expected_current_sha256.is_some_and(|expected| !hash.eq_ignore_ascii_case(expected))
            {
                return Err(format!(
                    "Rollback-Ziel {} hat unerwartete Pruefsumme {hash}",
                    target.display()
                ));
            }
        }
    }
    if let Err(error) = restore_existing(backup, target) {
        return recover_restore_error(
            backup,
            target,
            original_sha256,
            expected_current_sha256,
            error,
        );
    }
    verify_file_hash(target, original_sha256, "wiederhergestelltes Ziel")
}

fn recover_restore_error(
    backup: &Path,
    target: &Path,
    original_sha256: &str,
    expected_current_sha256: Option<&str>,
    operation_error: io::Error,
) -> Result<(), String> {
    match regular_file_hash(target, "Rollback-Ziel nach fehlgeschlagenem Ersetzen")? {
        Some(hash) if hash.eq_ignore_ascii_case(original_sha256) => Ok(()),
        Some(hash)
            if expected_current_sha256
                .is_some_and(|expected| hash.eq_ignore_ascii_case(expected)) =>
        {
            Err(format!(
                "{operation_error}; verifiziertes neues Ziel blieb unter {} lauffaehig",
                target.display()
            ))
        }
        None if verify_file_hash(backup, original_sha256, "Rollback-Sicherung").is_ok() => {
            rename_no_replace(backup, target)
                .map_err(|error| format!("{operation_error}; Notfall-Rollback: {error}"))?;
            verify_file_hash(target, original_sha256, "Notfall-Rollback-Ziel")
        }
        state => Err(format!(
            "{operation_error}; ungueltiger Rollback-Zustand: {state:?}"
        )),
    }
}
