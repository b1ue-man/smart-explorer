//! Handle-relative file primitives of the credential store directory: the
//! store opens its directory once and reaches every entry through `openat`,
//! `renameat` and `unlinkat` with `O_NOFOLLOW`, so a swapped path component
//! can never redirect a read, a write or a deletion.
use std::ffi::CString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(super) const DIRECTORY_MODE: u32 = 0o700;
pub(super) const FILE_MODE: u32 = 0o600;
const LOCK_NAME: &str = "store.lock";
const STAGE_ATTEMPTS: usize = 64;
pub(super) const STAGE_PREFIX: &str = ".stage-";

pub(super) struct StoreLock {
    _file: File,
}

/// The effective user the store belongs to. Tests may pretend to be another
/// user to exercise every ownership refusal without a second account.
#[cfg(not(test))]
pub(super) fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(test)]
thread_local! {
    pub(super) static UID_OVERRIDE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn effective_uid() -> u32 {
    UID_OVERRIDE
        .with(|forced| forced.get())
        .unwrap_or_else(|| unsafe { libc::geteuid() })
}

/// Opens the store directory, creating it (0700) when it does not exist.
pub(super) fn open_directory(path: &Path) -> Result<File, String> {
    let (directory, created) = match open_raw(path) {
        Ok(directory) => (directory, false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let created = create_directory(path)?;
            let directory =
                open_raw(path).map_err(|error| io_error("open credential directory", error))?;
            (directory, created)
        }
        Err(error) => return Err(io_error("open credential directory", error)),
    };
    if created {
        // Through the handle: a path swapped meanwhile can never be changed.
        directory
            .set_permissions(std::fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|error| io_error("set credential directory permissions", error))?;
    }
    validate_directory(&directory)?;
    Ok(directory)
}

/// Opens the store directory only when it exists (`None` otherwise); nothing
/// is created, so a process without anything to read leaves no trace.
pub(super) fn open_existing_directory(path: &Path) -> Result<Option<File>, String> {
    match open_raw(path) {
        Ok(directory) => {
            validate_directory(&directory)?;
            Ok(Some(directory))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("open credential directory", error)),
    }
}

fn open_raw(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

/// `true` when this call created the directory. Never creates a directory
/// inside a directory of another user (for example root with a foreign HOME).
fn create_directory(path: &Path) -> Result<bool, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let metadata = std::fs::metadata(parent)
            .map_err(|error| io_error("inspect credential directory parent", error))?;
        if metadata.uid() != effective_uid() {
            return Err(
                "Linux credential store: refusing to create the store inside a directory of another user"
                    .into(),
            );
        }
    }
    match DirBuilder::new().mode(DIRECTORY_MODE).create(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(io_error("create credential directory", error)),
    }
}

fn validate_directory(directory: &File) -> Result<(), String> {
    let metadata = directory
        .metadata()
        .map_err(|error| io_error("inspect credential directory", error))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != DIRECTORY_MODE
    {
        return Err(
            "Linux credential store: directory must be user-owned mode 0700 and not a link".into(),
        );
    }
    Ok(())
}

pub(super) fn acquire_lock(directory: &File) -> Result<StoreLock, String> {
    let name = c_name(LOCK_NAME)?;
    let exclusive_flags = libc::O_RDWR
        | libc::O_CREAT
        | libc::O_EXCL
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK;
    let (fd, created) = match openat(directory.as_raw_fd(), &name, exclusive_flags, FILE_MODE) {
        Ok(fd) => (fd, true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
            openat(
                directory.as_raw_fd(),
                &name,
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0,
            )
            .map_err(|error| io_error("open credential lock", error))?,
            false,
        ),
        Err(error) => return Err(io_error("create credential lock", error)),
    };
    let file = unsafe { File::from_raw_fd(fd) };
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(FILE_MODE))
            .map_err(|error| io_error("set credential lock permissions", error))?;
    }
    validate_secure_file(&file, "credential lock")?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io_error(
            "lock credential store",
            io::Error::last_os_error(),
        ));
    }
    validate_secure_file(&file, "credential lock")?;
    Ok(StoreLock { _file: file })
}

/// Opens an existing entry read-only, or read-write when `writable`.
pub(super) fn open_entry(
    directory: &File,
    name: &str,
    writable: bool,
) -> Result<Option<File>, String> {
    let name = c_name(name)?;
    let access = if writable {
        libc::O_RDWR
    } else {
        libc::O_RDONLY
    };
    match openat(
        directory.as_raw_fd(),
        &name,
        access | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        0,
    ) {
        Ok(fd) => Ok(Some(unsafe { File::from_raw_fd(fd) })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("open credential record", error)),
    }
}

pub(super) fn validate_record_file(file: &File, label: &str) -> Result<usize, String> {
    validate_secure_file(file, label)?;
    let length = file
        .metadata()
        .map_err(|error| io_error(&format!("inspect {label}"), error))?
        .len();
    usize::try_from(length)
        .map_err(|_| format!("Linux credential store: {label} length does not fit this platform"))
}

pub(super) fn validate_secure_file(file: &File, label: &str) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| io_error(&format!("inspect {label}"), error))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(format!(
            "Linux credential store: {label} must be a single-link, user-owned mode 0600 regular file"
        ));
    }
    Ok(())
}

/// Reads a validated entry completely, refusing more than `max_bytes`.
pub(super) fn read_entry(file: &File, label: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let size = validate_record_file(file, label)?;
    if size > max_bytes {
        return Err(format!(
            "Linux credential store: record exceeds {max_bytes} bytes"
        ));
    }
    let mut bytes = Vec::with_capacity(size);
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error(&format!("read {label}"), error))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "Linux credential store: record exceeds {max_bytes} bytes"
        ));
    }
    Ok(bytes)
}

/// Writes `bytes` to a fresh private stage file, syncs it and promotes it
/// over `final_name`. A failed write removes the stage; the old entry stays.
pub(super) fn write_entry(directory: &File, final_name: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let (stage_name, mut stage) = create_stage(directory)?;
    let mut promoted = false;
    let result = (|| {
        stage
            .write_all(bytes)
            .map_err(|error| io_error("write staged record", error))?;
        stage
            .sync_all()
            .map_err(|error| io_error("sync staged record", error))?;
        validate_secure_file(&stage, "staged record")?;
        drop(stage);
        rename_in(directory, &stage_name, final_name)?;
        promoted = true;
        directory
            .sync_all()
            .map_err(|error| io_error("sync credential directory", error))
    })();
    if result.is_err() && !promoted {
        let _ = unlink_in(directory, &stage_name);
    }
    result
}

/// Best effort: overwrites the content of an already replaced or unlinked
/// file through its open handle. On flash storage and copy-on-write file
/// systems the old blocks can survive; the plaintext is gone from every name.
pub(super) fn scrub(file: &File) {
    let Ok(metadata) = file.metadata() else {
        return;
    };
    let zeros = [0u8; 4096];
    let mut offset = 0u64;
    while offset < metadata.len() {
        let chunk = (metadata.len() - offset).min(zeros.len() as u64) as usize;
        if file.write_all_at(&zeros[..chunk], offset).is_err() {
            return;
        }
        offset += chunk as u64;
    }
    let _ = file.sync_all();
}

fn create_stage(directory: &File) -> Result<(String, File), String> {
    for _ in 0..STAGE_ATTEMPTS {
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce)
            .map_err(|error| format!("Linux credential store: generate staging nonce: {error}"))?;
        let name = format!(
            "{STAGE_PREFIX}{}-{}",
            std::process::id(),
            crate::creds::sealed_record::hex(&nonce)
        );
        let c_name = c_name(&name)?;
        match openat(
            directory.as_raw_fd(),
            &c_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            FILE_MODE,
        ) {
            Ok(fd) => {
                let file = unsafe { File::from_raw_fd(fd) };
                let setup = file
                    .set_permissions(std::fs::Permissions::from_mode(FILE_MODE))
                    .map_err(|error| io_error("set staged record permissions", error))
                    .and_then(|()| validate_secure_file(&file, "staged record"));
                match setup {
                    Ok(()) => return Ok((name, file)),
                    Err(error) => {
                        drop(file);
                        let _ = unlink_in(directory, &name);
                        return Err(error);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create staged record", error)),
        }
    }
    Err("Linux credential store: could not allocate a unique staging file".into())
}

fn openat(directory: RawFd, name: &CString, flags: i32, mode: u32) -> io::Result<RawFd> {
    let fd = unsafe { libc::openat(directory, name.as_ptr(), flags, mode) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn rename_in(directory: &File, source: &str, destination: &str) -> Result<(), String> {
    let source = c_name(source)?;
    let destination = c_name(destination)?;
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    } != 0
    {
        return Err(io_error(
            "promote credential record",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

pub(super) fn unlink_in(directory: &File, name: &str) -> Result<(), String> {
    let name = c_name(name)?;
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::NotFound {
            return Err(io_error("remove credential record", error));
        }
    }
    Ok(())
}

fn c_name(name: &str) -> Result<CString, String> {
    CString::new(name)
        .map_err(|_| "Linux credential store: generated filename contains NUL".to_string())
}

pub(super) fn io_error(action: &str, error: io::Error) -> String {
    format!("Linux credential store: {action}: {error}")
}
