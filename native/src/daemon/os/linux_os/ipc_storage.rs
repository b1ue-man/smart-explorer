use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const IPC_ADDR_FILE: &str = "daemon.ipc";
const IPC_GENERATION_FILE: &str = "daemon.generation";
const SYNC_DIRECTORY: &[u8] = b"sync\0";
const TOKEN_NAME: &[u8] = b"daemon.token\0";
const APP_DIRECTORY_MODE: u32 = 0o700;
const TOKEN_MODE: u32 = 0o600;
const MAX_TOKEN_BYTES: u64 = 4 * 1024;

fn ipc_addr_path() -> PathBuf {
    app_data_path().join("sync").join(IPC_ADDR_FILE)
}

fn ipc_generation_path() -> PathBuf {
    app_data_path().join("sync").join(IPC_GENERATION_FILE)
}

pub(super) fn clear_ipc_addr() {
    if secure_sync_directory().is_ok() {
        let _ = std::fs::remove_file(ipc_addr_path());
    }
}

pub(super) fn clear_ipc_generation() {
    if secure_sync_directory().is_ok() {
        let _ = std::fs::remove_file(ipc_generation_path());
    }
}

pub(super) fn write_ipc_addr(addr: SocketAddr) -> io::Result<()> {
    secure_sync_directory()?;
    std::fs::write(ipc_addr_path(), addr.to_string())
}

pub(super) fn write_ipc_generation(generation: &str) -> io::Result<()> {
    secure_sync_directory()?;
    std::fs::write(ipc_generation_path(), generation)
}

pub(super) fn read_ipc_addr() -> Option<SocketAddr> {
    secure_sync_directory().ok()?;
    std::fs::read_to_string(ipc_addr_path())
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

pub(super) fn read_ipc_generation() -> Option<String> {
    secure_sync_directory().ok()?;
    let generation = std::fs::read_to_string(ipc_generation_path()).ok()?;
    let generation = generation.trim();
    (generation.len() == 32 && generation.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| generation.to_string())
}

pub(super) fn load_or_create_token() -> io::Result<String> {
    load_or_create_token_in(&app_data_path())
}

pub(super) fn read_token() -> io::Result<String> {
    read_token_in(&app_data_path())
}

fn app_data_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

fn secure_sync_directory() -> io::Result<File> {
    secure_sync_directory_in(&app_data_path())
}

fn secure_sync_directory_in(app_path: &Path) -> io::Result<File> {
    let app = open_directory(app_path)?;
    enforce_directory_mode(&app, "application data directory")?;

    let created = unsafe {
        libc::mkdirat(
            app.as_raw_fd(),
            SYNC_DIRECTORY.as_ptr().cast(),
            APP_DIRECTORY_MODE,
        )
    };
    if created != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(error);
        }
    }

    let fd = unsafe {
        libc::openat(
            app.as_raw_fd(),
            SYNC_DIRECTORY.as_ptr().cast(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let sync = unsafe { File::from_raw_fd(fd) };
    enforce_directory_mode(&sync, "IPC data directory")?;
    Ok(sync)
}

fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn enforce_directory_mode(directory: &File, label: &str) -> io::Result<()> {
    let metadata = directory.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_dir() || metadata.uid() != uid {
        return Err(permission_denied(format!(
            "{label} must be a user-owned directory"
        )));
    }

    directory.set_permissions(std::fs::Permissions::from_mode(APP_DIRECTORY_MODE))?;
    let metadata = directory.metadata()?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != APP_DIRECTORY_MODE
    {
        return Err(permission_denied(format!(
            "{label} must be user-owned mode 0700"
        )));
    }
    Ok(())
}

fn load_or_create_token_in(app_path: &Path) -> io::Result<String> {
    let directory = secure_sync_directory_in(app_path)?;
    match open_existing_token(&directory) {
        Ok(mut file) => read_token_file(&mut file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_token(&directory),
        Err(error) => Err(error),
    }
}

fn read_token_in(app_path: &Path) -> io::Result<String> {
    let directory = secure_sync_directory_in(app_path)?;
    let mut file = open_existing_token(&directory)?;
    read_token_file(&mut file)
}

fn open_existing_token(directory: &File) -> io::Result<File> {
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            TOKEN_NAME.as_ptr().cast(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    enforce_token_mode(&file)?;
    Ok(file)
}

fn create_token(directory: &File) -> io::Result<String> {
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            TOKEN_NAME.as_ptr().cast(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            TOKEN_MODE,
        )
    };
    if fd < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::AlreadyExists {
            let mut file = open_existing_token(directory)?;
            return read_token_file(&mut file);
        }
        return Err(error);
    }

    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        enforce_token_mode(&file)?;
        let token = generate_token()?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        let stored = read_token_file(&mut file)?;
        if stored != token {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "daemon IPC token write verification failed",
            ));
        }
        Ok(stored)
    })();
    if result.is_err() {
        let _ = unsafe { libc::unlinkat(directory.as_raw_fd(), TOKEN_NAME.as_ptr().cast(), 0) };
    }
    result
}

fn enforce_token_mode(file: &File) -> io::Result<()> {
    validate_token_identity(file)?;
    file.set_permissions(std::fs::Permissions::from_mode(TOKEN_MODE))?;
    let metadata = validate_token_identity(file)?;
    if metadata.mode() & 0o7777 != TOKEN_MODE {
        return Err(permission_denied(
            "daemon IPC token must be user-owned mode 0600",
        ));
    }
    Ok(())
}

fn validate_token_identity(file: &File) -> io::Result<std::fs::Metadata> {
    let metadata = file.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_file() || metadata.uid() != uid || metadata.nlink() != 1 {
        return Err(permission_denied(
            "daemon IPC token must be a single-link, user-owned regular file",
        ));
    }
    Ok(metadata)
}

fn read_token_file(file: &mut File) -> io::Result<String> {
    let metadata = file.metadata()?;
    if metadata.len() > MAX_TOKEN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon IPC token file too large",
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    validate_token_text(&text)
}

fn validate_token_text(text: &str) -> io::Result<String> {
    let token = text.trim();
    if token.len() < 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon IPC token too short",
        ));
    }
    Ok(token.to_string())
}

fn generate_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn permission_denied(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}

#[cfg(test)]
mod tests {
    use super::{load_or_create_token_in, read_token_in, secure_sync_directory_in};
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn app_directory() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        std::fs::create_dir(&app).unwrap();
        (root, app)
    }

    #[test]
    fn storage_enforces_private_directory_and_token_modes() {
        let (_root, app) = app_directory();
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();

        let token = load_or_create_token_in(&app).unwrap();

        assert_eq!(
            std::fs::metadata(&app).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(app.join("sync"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(app.join("sync/daemon.token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
        assert_eq!(read_token_in(&app).unwrap(), token);
    }

    #[test]
    fn existing_token_permissions_are_tightened_then_revalidated() {
        let (_root, app) = app_directory();
        secure_sync_directory_in(&app).unwrap();
        let path = app.join("sync/daemon.token");
        std::fs::write(&path, "a".repeat(64)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(read_token_in(&app).unwrap(), "a".repeat(64));
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn token_symlink_fails_closed() {
        let (root, app) = app_directory();
        secure_sync_directory_in(&app).unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, "a".repeat(64)).unwrap();
        symlink(target, app.join("sync/daemon.token")).unwrap();

        assert!(read_token_in(&app).is_err());
    }

    #[test]
    fn hard_linked_token_fails_closed() {
        let (_root, app) = app_directory();
        secure_sync_directory_in(&app).unwrap();
        let token = app.join("sync/daemon.token");
        std::fs::write(&token, "a".repeat(64)).unwrap();
        std::fs::hard_link(&token, app.join("sync/other-link")).unwrap();

        assert!(read_token_in(&app).is_err());
    }

    #[test]
    fn invalid_existing_token_is_not_replaced() {
        let (_root, app) = app_directory();
        secure_sync_directory_in(&app).unwrap();
        let token = app.join("sync/daemon.token");
        std::fs::write(&token, "short").unwrap();

        assert!(load_or_create_token_in(&app).is_err());
        assert_eq!(std::fs::read_to_string(token).unwrap(), "short");
    }
}
