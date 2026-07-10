use sha2::Digest;
use std::path::Path;

pub(crate) struct UpdateInstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(target_os = "linux")]
    _file: std::fs::File,
}

/// Serialize helpers for one installed target. v0.5.119 can launch several
/// workers while its update dialog remains open, so every worker must join the
/// same boundary before it evaluates or changes the installed winner.
pub(crate) fn acquire(target: &Path) -> Result<UpdateInstanceGuard, String> {
    let key = target_key(target);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED_0, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject, INFINITE};

        let name: Vec<u16> =
            std::ffi::OsStr::new(&format!(r"Local\SmartExplorerUpdaterApply-{key}"))
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(format!(
                    "Updater-Sperre anlegen: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let result = WaitForSingleObject(handle, INFINITE);
            if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED_0 {
                CloseHandle(handle);
                return Err(format!(
                    "Auf Updater-Sperre warten: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(UpdateInstanceGuard { handle })
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        let path = super::logging::appdata_dir().join(format!("updater-{key}.lock"));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|error| format!("Updater-Sperre {} oeffnen: {error}", path.display()))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Updater-Sperre {} pruefen: {error}", path.display()))?;
        if !metadata.file_type().is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err(format!(
                "Updater-Sperre {} ist keine eindeutige benutzereigene Datei",
                path.display()
            ));
        }
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(format!(
                    "Updater-Sperre {} belegen: {error}",
                    path.display()
                ));
            }
        }
        Ok(UpdateInstanceGuard { _file: file })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    Err("Updater-Sperre wird auf diesem Betriebssystem nicht unterstuetzt".to_string())
}

pub(crate) fn target_key(target: &Path) -> String {
    let absolute = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(target))
            .unwrap_or_else(|_| target.to_path_buf())
    };
    let resolved = std::fs::canonicalize(&absolute).unwrap_or_else(|_| {
        absolute
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .and_then(|parent| absolute.file_name().map(|name| parent.join(name)))
            .unwrap_or(absolute)
    });
    let mut hasher = sha2::Sha256::new();
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut wide = resolved.as_os_str().encode_wide().collect::<Vec<_>>();
        let extended = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        if wide.starts_with(&extended) {
            wide.drain(..extended.len());
            let unc = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
            if wide.starts_with(&unc) {
                wide.splice(..unc.len(), [b'\\' as u16, b'\\' as u16]);
            }
        }
        for mut unit in wide {
            if unit == b'/' as u16 {
                unit = b'\\' as u16;
            } else if (b'A' as u16..=b'Z' as u16).contains(&unit) {
                unit += (b'a' - b'A') as u16;
            }
            hasher.update(unit.to_le_bytes());
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(resolved.as_os_str().as_bytes());
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    hasher.update(resolved.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(windows)]
impl Drop for UpdateInstanceGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        unsafe {
            let _ = ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_key_is_stable_for_same_path() {
        let path = std::env::temp_dir().join("smart-explorer-updater-target");
        assert_eq!(target_key(&path), target_key(&path));
        assert_eq!(target_key(&path).len(), 64);
    }

    #[cfg(unix)]
    #[test]
    fn target_key_survives_missing_file_under_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let target = alias.join("app");
        std::fs::write(&target, b"app").unwrap();
        let existing = target_key(&target);
        std::fs::remove_file(&target).unwrap();

        assert_eq!(target_key(&target), existing);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helpers_for_same_target_are_serialized() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"app").unwrap();
        let first = acquire(&target).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let second_target = target.clone();
        let waiter = std::thread::spawn(move || {
            let guard = acquire(&second_target).unwrap();
            tx.send(()).unwrap();
            guard
        });

        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        drop(waiter.join().unwrap());
    }
}
