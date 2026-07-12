use std::borrow::Cow;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[cfg(test)]
const DAEMON_LOCK_FILE: &str = "smart-explorer-sync-daemon.lock";

#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
}

/// Keeps the open file description (and therefore its `flock`) alive.
pub(crate) struct DaemonInstanceGuard {
    _lock_file: File,
}

pub(crate) fn removable_drives() -> Vec<DriveInfo> {
    Vec::new()
}

pub(crate) fn battery_saver_on() -> bool {
    false
}

pub(crate) fn on_metered_network() -> bool {
    false
}

pub(crate) fn run_shell_command(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("sh").args(["-c", cmd]).status()
}

pub(crate) fn normalize_local_backend_path(path: &str) -> Cow<'_, str> {
    Cow::Borrowed(path)
}

pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "control path has no parent"))?;
    File::open(parent)?.sync_all()
}

pub(crate) fn restore_control_if_absent(source: &Path, destination: &Path) -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;

    let source_c = std::ffi::CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination_c = std::ffi::CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let renamed = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        return Ok(false);
    }
    if !error
        .raw_os_error()
        .is_some_and(|code| matches!(code, libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP))
    {
        return Err(error);
    }
    match std::fs::hard_link(source, destination) {
        Ok(()) => {
            std::fs::remove_file(source)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn metadata_is_link_like(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn acquire_daemon_instance_guard(
    timeout: std::time::Duration,
) -> Option<DaemonInstanceGuard> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match daemon_lock_directory().and_then(|directory| try_acquire_in(&directory)) {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => return None,
            Err(error) => {
                super::state::log(&format!("daemon single-instance lock failed: {error}"));
                return None;
            }
        }
    }
}

fn daemon_lock_directory() -> io::Result<PathBuf> {
    if let Some(raw) = std::env::var_os("XDG_RUNTIME_DIR").filter(|raw| !raw.is_empty()) {
        let directory = PathBuf::from(raw);
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "XDG_RUNTIME_DIR must be absolute",
            ));
        }
        return Ok(directory);
    }

    let uid = unsafe { libc::geteuid() };
    let standard = PathBuf::from(format!("/run/user/{uid}"));
    if standard.exists() {
        return Ok(standard);
    }

    let fallback = PathBuf::from(format!("/tmp/smart-explorer-runtime-{uid}"));
    match DirBuilder::new().mode(0o700).create(&fallback) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    Ok(fallback)
}

fn try_acquire_in(directory: &Path) -> io::Result<Option<DaemonInstanceGuard>> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?;
    validate_directory(&directory)?;

    let name = b"smart-explorer-sync-daemon.lock\0";
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr().cast(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let lock_file = unsafe { File::from_raw_fd(fd) };
    validate_lock_file(&lock_file)?;

    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(DaemonInstanceGuard {
            _lock_file: lock_file,
        }));
    }

    let error = io::Error::last_os_error();
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
    {
        Ok(None)
    } else {
        Err(error)
    }
}

fn validate_directory(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon lock directory must be user-owned mode 0700",
        ));
    }
    Ok(())
}

fn validate_lock_file(lock_file: &File) -> io::Result<()> {
    let metadata = lock_file.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon lock file must be a single-link, user-owned mode 0600 regular file",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_local_backend_path, try_acquire_in, DAEMON_LOCK_FILE};
    use std::os::unix::fs::{symlink, PermissionsExt};

    const CHILD_DIRECTORY: &str = "SMART_EXPLORER_TEST_DAEMON_LOCK_DIRECTORY";
    const CHILD_EXPECT_LOCKED: &str = "SMART_EXPLORER_TEST_DAEMON_LOCK_EXPECT_LOCKED";

    fn secure_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    #[test]
    fn local_backend_paths_preserve_valid_backslash_characters() {
        assert_eq!(
            normalize_local_backend_path(r"/tmp/name\with-backslash"),
            r"/tmp/name\with-backslash"
        );
    }

    #[test]
    fn guard_excludes_an_independent_open_until_drop() {
        let directory = secure_directory();
        let first = try_acquire_in(directory.path()).unwrap().unwrap();
        assert!(try_acquire_in(directory.path()).unwrap().is_none());
        drop(first);
        assert!(try_acquire_in(directory.path()).unwrap().is_some());
    }

    #[test]
    fn guard_excludes_a_second_process_until_drop() {
        let directory = secure_directory();
        let first = try_acquire_in(directory.path()).unwrap().unwrap();
        assert!(run_child_probe(directory.path(), true).success());
        drop(first);
        assert!(run_child_probe(directory.path(), false).success());
    }

    #[test]
    fn linux_daemon_lock_probe_child_only() {
        let Some(directory) = std::env::var_os(CHILD_DIRECTORY) else {
            return;
        };
        let expected_locked =
            std::env::var_os(CHILD_EXPECT_LOCKED).is_some_and(|value| value == "1");
        let guard = try_acquire_in(std::path::Path::new(&directory)).unwrap();
        assert_eq!(guard.is_none(), expected_locked);
    }

    fn run_child_probe(
        directory: &std::path::Path,
        expected_locked: bool,
    ) -> std::process::ExitStatus {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("linux_daemon_lock_probe_child_only")
            .arg("--test-threads=1")
            .env(CHILD_DIRECTORY, directory)
            .env(CHILD_EXPECT_LOCKED, if expected_locked { "1" } else { "0" })
            .status()
            .unwrap()
    }

    #[test]
    fn lock_file_symlink_fails_closed() {
        let directory = secure_directory();
        let target = directory.path().join("target");
        std::fs::write(&target, b"not a lock").unwrap();
        symlink(&target, directory.path().join(DAEMON_LOCK_FILE)).unwrap();
        assert!(try_acquire_in(directory.path()).is_err());
    }

    #[test]
    fn insecure_lock_file_permissions_fail_closed() {
        let directory = secure_directory();
        let lock = directory.path().join(DAEMON_LOCK_FILE);
        std::fs::write(&lock, b"").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(try_acquire_in(directory.path()).is_err());
    }

    #[test]
    fn insecure_lock_directory_permissions_fail_closed() {
        let directory = secure_directory();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(try_acquire_in(directory.path()).is_err());
    }
}
