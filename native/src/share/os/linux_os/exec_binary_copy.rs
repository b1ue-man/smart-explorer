//! The program file of the account units and of the account service (C3):
//! Smart Explorer's own file where that file and every directory above it
//! belong to root and neither group nor others may write them, else the
//! root-owned copy `/usr/local/sbin/smart-explorer-exec` (SELinux type
//! `bin_t` under Fedora). Before every start the file is checked against the
//! SHA-256 of the running image, so a unit never runs another Smart Explorer
//! than the worker or service that starts it. The file also holds the checks
//! and atomic writes of the other root files of the account service (PAM
//! service, configuration, unit files) and the owner-only runtime directory
//! of the worker's own supervisor sockets. No check follows a symbolic link.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::share::exec_types::ExecId;

pub(super) const PROGRAM_COPY: &str = "/usr/local/sbin/smart-explorer-exec";
const RUNNING_IMAGE: &str = "/proc/self/exe";
/// Mode bits that let group or others write.
const FOREIGN_WRITE: u32 = 0o022;

/// Opens `path` for reading without following a symbolic link: every
/// directory above it and the regular file itself belong to root, and
/// neither group nor others may write them.
pub(super) fn opened_trusted(path: &Path) -> io::Result<File> {
    let directory = path
        .parent()
        .ok_or_else(|| untrusted(path, "has no directory"))?;
    trusted_directories(directory)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.uid() != 0
        || metadata.mode() & FOREIGN_WRITE != 0
    {
        return Err(untrusted(
            path,
            "is not a regular file that only root can change",
        ));
    }
    Ok(file)
}

/// Every directory from `directory` up to `/` is a directory of root, no
/// symbolic link, that neither group nor others may write.
pub(super) fn trusted_directories(directory: &Path) -> io::Result<()> {
    if !directory.is_absolute() {
        return Err(untrusted(directory, "is not an absolute path"));
    }
    for ancestor in directory.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.file_type().is_dir()
            || metadata.uid() != 0
            || metadata.mode() & FOREIGN_WRITE != 0
        {
            return Err(untrusted(
                ancestor,
                "is not a directory that only root can change",
            ));
        }
    }
    Ok(())
}

/// Creates `directory` as root's with mode 0755 where it is missing; its
/// parents must be trusted, and so must it afterwards.
pub(super) fn ensure_root_directory(directory: &Path) -> io::Result<()> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = directory
                .parent()
                .ok_or_else(|| untrusted(directory, "has no parent"))?;
            trusted_directories(parent)?;
            fs::DirBuilder::new().mode(0o755).create(directory)?;
            std::os::unix::fs::lchown(directory, Some(0), Some(0))?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    trusted_directories(directory)
}

/// Writes `contents` as root's file `path` with `mode`: a new file in the
/// same directory (so it gets that directory's SELinux label), synced, then
/// renamed over `path`. Readers see the old or the new file, never a part.
pub(super) fn write_root_file(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| untrusted(path, "has no directory"))?;
    trusted_directories(directory)?;
    let (temporary, mut file) = create_temporary(path)?;
    let written = write_contents(&mut file, contents, mode);
    commit_temporary(&temporary, path, written)
}

/// SHA-256 (hex) of a file of root (see `opened_trusted`).
pub(super) fn trusted_sha256(path: &Path) -> io::Result<String> {
    hash_reader(opened_trusted(path)?)
}

/// SHA-256 (hex) of the running program image. The image cannot change
/// while the process runs (`/proc/self/exe` keeps a replaced file), so it is
/// computed once.
pub(super) fn running_image_sha256() -> io::Result<String> {
    static IMAGE: OnceLock<String> = OnceLock::new();
    if let Some(hash) = IMAGE.get() {
        return Ok(hash.clone());
    }
    let hash = hash_reader(File::open(image_source())?)?;
    Ok(IMAGE.get_or_init(|| hash).clone())
}

/// The program of a root worker's account units (PRM-M5): the worker's own
/// file where it is trusted and still holds the running image, else the
/// copy, renewed where it differs.
pub(super) fn program_for_units() -> io::Result<PathBuf> {
    let image = running_image_sha256()?;
    if let Some(path) = image_path() {
        if trusted_sha256(&path).is_ok_and(|hash| hash == image) {
            return Ok(path);
        }
    }
    let copy = Path::new(PROGRAM_COPY);
    if trusted_sha256(copy).is_ok_and(|hash| hash == image) {
        return Ok(copy.to_path_buf());
    }
    install_copy()?;
    Ok(copy.to_path_buf())
}

/// Copies the running image to `PROGRAM_COPY` (root only) and returns its
/// SHA-256; a unit that still runs the old copy keeps its file.
pub(super) fn install_copy() -> io::Result<String> {
    let copy = Path::new(PROGRAM_COPY);
    let directory = copy
        .parent()
        .ok_or_else(|| untrusted(copy, "has no directory"))?;
    ensure_root_directory(directory)?;
    let image = running_image_sha256()?;
    let (temporary, mut file) = create_temporary(copy)?;
    let written = copy_image(&mut file);
    commit_temporary(&temporary, copy, written)?;
    let copied = trusted_sha256(copy)?;
    if copied != image {
        return Err(io::Error::other(format!(
            "{PROGRAM_COPY} differs from the running program after copying"
        )));
    }
    Ok(copied)
}

/// The program of the units the account service starts: the service's own
/// file, trusted and still holding the running image. A replaced copy means
/// an update came after this instance started.
pub(super) fn helper_program() -> io::Result<PathBuf> {
    let path = image_path().ok_or_else(|| {
        io::Error::other("the program file of the account service was replaced or removed")
    })?;
    if trusted_sha256(&path)? != running_image_sha256()? {
        return Err(io::Error::other(format!(
            "{} differs from the running account service",
            path.display()
        )));
    }
    Ok(path)
}

/// Removes the copy (root only); `false` when there was none.
pub(super) fn remove_copy() -> io::Result<bool> {
    match fs::remove_file(PROGRAM_COPY) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// The worker's own directory for the sockets its own account's supervisors
/// connect to: `/run/user/<uid>/smart-explorer-exec`, else
/// `/tmp/smart-explorer-runtime-<uid>/smart-explorer-exec`, both owner-only.
pub(super) fn runtime_directory() -> io::Result<PathBuf> {
    // SAFETY: `geteuid` only reads this process's id.
    let uid = unsafe { libc::geteuid() };
    let base = PathBuf::from(format!("/run/user/{uid}"));
    let base = if base.is_dir() {
        base
    } else {
        PathBuf::from(format!("/tmp/smart-explorer-runtime-{uid}"))
    };
    match std::fs::create_dir(&base) {
        Ok(()) => std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_owner_directory(&base, uid, "exec runtime base")?;
    let directory = base.join("smart-explorer-exec");
    match std::fs::create_dir(&directory) {
        Ok(()) => std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_owner_directory(&directory, uid, "exec runtime directory")?;
    Ok(directory)
}

fn validate_owner_directory(path: &Path, uid: u32, label: &str) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && metadata.uid() == uid
        && metadata.mode() & 0o777 == 0o700
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{label} must be owner-only mode 0700"),
        ))
    }
}

/// The image this process runs. The tests of the remote suite name the
/// built `se` instead (`SMART_EXPLORER_PERM_SE_BIN`): the test program has
/// no supervisor mode.
fn image_source() -> PathBuf {
    test_image().unwrap_or_else(|| PathBuf::from(RUNNING_IMAGE))
}

/// The path of the running image's file; `None` where it was replaced or
/// removed since this process started.
fn image_path() -> Option<PathBuf> {
    if let Some(path) = test_image() {
        return Some(path);
    }
    let path = fs::read_link(RUNNING_IMAGE).ok()?;
    let replaced = path
        .to_str()
        .is_some_and(|text| text.ends_with(" (deleted)"));
    (!replaced).then_some(path)
}

#[cfg(test)]
fn test_image() -> Option<PathBuf> {
    std::env::var_os("SMART_EXPLORER_PERM_SE_BIN").map(PathBuf::from)
}

#[cfg(not(test))]
fn test_image() -> Option<PathBuf> {
    None
}

fn hash_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn create_temporary(path: &Path) -> io::Result<(PathBuf, File)> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| untrusted(path, "has no file name"))?;
    let temporary = path.with_file_name(format!(".{name}.{}", ExecId::generate()?.as_str()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    Ok((temporary, file))
}

fn write_contents(file: &mut File, contents: &[u8], mode: u32) -> io::Result<()> {
    file.write_all(contents)?;
    finish_root_file(file, mode)
}

fn copy_image(file: &mut File) -> io::Result<()> {
    let mut source = File::open(image_source())?;
    io::copy(&mut source, file)?;
    finish_root_file(file, 0o755)
}

fn finish_root_file(file: &File, mode: u32) -> io::Result<()> {
    std::os::unix::fs::fchown(file, Some(0), Some(0))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()
}

fn commit_temporary(temporary: &Path, path: &Path, written: io::Result<()>) -> io::Result<()> {
    let result = written
        .and_then(|()| fs::rename(temporary, path))
        .and_then(|()| sync_directory(path));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn sync_directory(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(directory) => File::open(directory)?.sync_all(),
        None => Ok(()),
    }
}

fn untrusted(path: &Path, what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} {what}", path.display()),
    )
}
