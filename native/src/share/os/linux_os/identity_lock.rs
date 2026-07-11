use std::ffi::CString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

const LOCK_DIRECTORY: &str = "identity-lock-v1";
const LOCK_FILE: &str = "transaction.lock";
const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

pub(super) struct IdentityLock {
    _directory: File,
    _file: File,
}

pub(super) fn acquire(app_data_dir: &Path) -> io::Result<IdentityLock> {
    std::fs::create_dir_all(app_data_dir)?;
    let directory = open_lock_directory(app_data_dir)?;
    validate_directory(&directory)?;
    let file = open_lock_file(&directory)?;
    validate_lock_file(&file)?;
    flock_exclusive(&file)?;
    validate_directory(&directory)?;
    validate_lock_file(&file)?;
    Ok(IdentityLock {
        _directory: directory,
        _file: file,
    })
}

fn open_lock_directory(app_data_dir: &Path) -> io::Result<File> {
    let path = app_data_dir.join(LOCK_DIRECTORY);
    let created = match DirBuilder::new().mode(DIRECTORY_MODE).create(&path) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error),
    };
    if created {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(DIRECTORY_MODE))?;
    }
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

fn open_lock_file(directory: &File) -> io::Result<File> {
    let name = c_name(LOCK_FILE)?;
    let create_flags = libc::O_RDWR
        | libc::O_CREAT
        | libc::O_EXCL
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK;
    let (fd, created) = match openat(directory.as_raw_fd(), &name, create_flags, FILE_MODE) {
        Ok(fd) => (fd, true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
            openat(
                directory.as_raw_fd(),
                &name,
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0,
            )?,
            false,
        ),
        Err(error) => return Err(error),
    };
    let file = unsafe { File::from_raw_fd(fd) };
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))?;
    }
    Ok(file)
}

fn validate_directory(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != DIRECTORY_MODE
    {
        return Err(permission_denied(
            "Share identity lock directory must be user-owned mode 0700 and not a link",
        ));
    }
    Ok(())
}

fn validate_lock_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(permission_denied(
            "Share identity lock must be a single-link, user-owned mode 0600 regular file",
        ));
    }
    Ok(())
}

fn flock_exclusive(file: &File) -> io::Result<()> {
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn openat(directory: RawFd, name: &CString, flags: i32, mode: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::openat(directory, name.as_ptr(), flags, mode) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid lock name"))
}

fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn permission_denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn lock_path(app_data_dir: &Path) -> std::path::PathBuf {
        app_data_dir.join(LOCK_DIRECTORY).join(LOCK_FILE)
    }

    #[test]
    fn symlink_lock_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app");
        let directory = app_data.join(LOCK_DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(DIRECTORY_MODE))
            .unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(target, lock_path(&app_data)).unwrap();

        assert!(acquire(&app_data).is_err());
    }

    #[test]
    fn insecure_lock_mode_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app");
        drop(acquire(&app_data).unwrap());
        std::fs::set_permissions(lock_path(&app_data), std::fs::Permissions::from_mode(0o644))
            .unwrap();

        assert!(acquire(&app_data).is_err());
    }

    #[test]
    fn insecure_directory_mode_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app");
        drop(acquire(&app_data).unwrap());
        std::fs::set_permissions(
            app_data.join(LOCK_DIRECTORY),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        assert!(acquire(&app_data).is_err());
    }

    #[test]
    fn concurrent_acquirers_are_serialized() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app");
        let first = acquire(&app_data).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let contender_path = app_data.clone();
        let contender = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let lock = acquire(&contender_path);
            acquired_tx.send(lock.is_ok()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(first);
        assert!(acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap());
        contender.join().unwrap();
    }
}
