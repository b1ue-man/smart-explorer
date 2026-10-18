//! Where a Linux drive appears and the rules at that folder (DRV-M4,
//! decision C2 "Einhängeort", S3 OE2).
//!
//! Drives appear below `~/SmartExplorer/` (GLib, GVfs, Nautilus, Nemo and
//! Thunar show mounts below `$HOME` by themselves, named after the last path
//! part), vault drives below `$XDG_RUNTIME_DIR/smart-explorer/Tresore/`: that
//! folder must lie on tmpfs, so a write after an unnoticed end lands in
//! memory, never on disk; without a login session (`XDG_RUNTIME_DIR`
//! missing) a vault drive is refused, there is no fallback to the disk.
//!
//! At the folder `fusermount3` requires write access and no symlink; se also
//! requires an empty plain folder owned by this user (se changes its mode,
//! and a mount would hide files in it). Outside a mount the folder is
//! locked to `0500`, so a program that writes there by mistake fails; it is
//! opened to `0700` right before the mount. A vault folder keeps the sticky
//! bit, which protects it from the periodic cleanup of the runtime folder
//! (XDG Base Directory specification). A dead FUSE mount (`ENOTCONN`, left
//! by a crashed host) is unmounted lazily before anything else.
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Mount type of every se drive (`-o subtype=smartexplorer`).
pub(crate) const FUSE_TYPE: &str = "fuse.smartexplorer";
pub(crate) const FUSE_SUBTYPE: &str = "smartexplorer";
const DRIVES_FOLDER: &str = "SmartExplorer";
const VAULTS_FOLDER: [&str; 2] = ["smart-explorer", "Tresore"];
const LOCKED: u32 = 0o500;
const OPEN: u32 = 0o700;
const STICKY: u32 = 0o1000;
const TMPFS_MAGIC: i128 = 0x0102_1994;
/// Highest suffix tried for a default folder name (`Name (2)` …).
const MAX_SUFFIX: usize = 999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceKind {
    Drive,
    Vault,
}

/// `~/SmartExplorer`.
pub(crate) fn drives_root() -> io::Result<PathBuf> {
    crate::support_dirs::home_dir()
        .filter(|home| home.is_absolute())
        .map(|home| home.join(DRIVES_FOLDER))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME ist nicht gesetzt"))
}

/// `$XDG_RUNTIME_DIR/smart-explorer/Tresore`.
pub(crate) fn vaults_root() -> io::Result<PathBuf> {
    vaults_root_in(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
}

pub(crate) fn vaults_root_in(runtime: Option<PathBuf>) -> io::Result<PathBuf> {
    let runtime = runtime.filter(|path| path.is_absolute()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Tresor-Laufwerke brauchen einen Arbeitsspeicher-Einhängeort: XDG_RUNTIME_DIR fehlt (keine Anmeldesitzung)",
        )
    })?;
    Ok(VAULTS_FOLDER
        .iter()
        .fold(runtime, |path, folder| path.join(folder)))
}

/// A folder below `$XDG_RUNTIME_DIR/smart-explorer/Tresore` follows the
/// vault rules (tmpfs, sticky bit), whatever the source of the drive.
pub(crate) fn place_kind(location: &Path) -> PlaceKind {
    if vaults_root().is_ok_and(|root| location.starts_with(root)) {
        PlaceKind::Vault
    } else {
        PlaceKind::Drive
    }
}

/// The folder name of a drive label: `/`, NUL and control characters become
/// `_`, leading dots go (a hidden folder would hide the drive), at most 255
/// bytes; an empty result becomes `Laufwerk`.
pub(crate) fn folder_name(label: &str) -> String {
    let mut name = String::new();
    for character in label.trim().chars() {
        let character = if character == '/' || character.is_control() {
            '_'
        } else {
            character
        };
        if name.len() + character.len_utf8() > 255 {
            break;
        }
        name.push(character);
    }
    let name = name.trim_start_matches('.').trim();
    if name.is_empty() {
        "Laufwerk".into()
    } else {
        name.to_string()
    }
}

/// `<root>/<name>`, or `<name> (2)`, `<name> (3)` … while `taken` says the
/// folder is in use.
pub(crate) fn default_location(
    root: &Path,
    label: &str,
    taken: impl Fn(&Path) -> bool,
) -> io::Result<PathBuf> {
    let name = folder_name(label);
    for index in 1..=MAX_SUFFIX {
        let candidate = if index == 1 {
            root.join(&name)
        } else {
            root.join(format!("{name} ({index})"))
        };
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "kein freier Einhängeort für dieses Laufwerk",
    ))
}

/// A folder another drive could use now: absent, or an empty plain folder
/// of this user that is no mount point.
pub(crate) fn folder_is_free(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Err(error) => error.kind() == io::ErrorKind::NotFound,
        Ok(metadata) => {
            metadata.is_dir()
                && metadata.uid() == effective_uid()
                && !is_mount_point(path)
                && fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
        }
    }
}

/// Checks and prepares the folder right before the host mounts: creates it
/// (with missing parents, `0700`), removes a dead se mount, then requires an
/// empty plain folder of this user that is no mount point, for a vault on
/// tmpfs; leaves it locked.
pub(crate) fn prepare(path: &Path, kind: PlaceKind) -> io::Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(invalid("Einhängeort muss ein absoluter Ordnerpfad sein"));
    }
    if dead_mount(path) {
        unmount(path, true)?;
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            DirBuilder::new().recursive(true).mode(OPEN).create(path)?;
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(refused(
            path,
            "ist kein Ordner (Verknüpfungen sind nicht erlaubt)",
        ));
    }
    if metadata.uid() != effective_uid() {
        return Err(refused(path, "gehört einem anderen Nutzer"));
    }
    if is_mount_point(path) {
        return Err(refused(path, "ist bereits ein Einhängepunkt"));
    }
    if fs::read_dir(path)?.next().is_some() {
        return Err(refused(path, "ist nicht leer"));
    }
    if kind == PlaceKind::Vault && !on_tmpfs(path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Tresor-Laufwerke brauchen einen Arbeitsspeicher-Einhängeort (tmpfs)",
        ));
    }
    set_mode(path, kind, LOCKED)
}

/// Opens the prepared folder for the mount (`0700`).
pub(crate) fn open_for_mount(path: &Path, kind: PlaceKind) -> io::Result<()> {
    set_mode(path, kind, OPEN)
}

/// Locks the folder again (`0500`) once nothing is mounted there.
pub(crate) fn lock(path: &Path, kind: PlaceKind) -> io::Result<()> {
    if is_mount_point(path) || dead_mount(path) {
        return Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            "der Einhängeort ist noch eingehängt",
        ));
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Ok(()),
        Ok(_) => set_mode(path, kind, LOCKED),
    }
}

/// The folder as `mountinfo` lists it: the canonical parent plus the name
/// (a dead mount point itself cannot be resolved).
pub(crate) fn canonical(path: &Path) -> io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(invalid("Einhängeort muss ein absoluter Ordnerpfad sein"));
    };
    Ok(fs::canonicalize(parent)?.join(name))
}

/// Something is mounted at `path` (another device than the parent, or a
/// `mountinfo` entry).
pub(crate) fn is_mount_point(path: &Path) -> bool {
    let canonical = canonical(path).unwrap_or_else(|_| path.to_path_buf());
    if super::fuse_abort::mount_at(&canonical).is_some() {
        return true;
    }
    match (fs::symlink_metadata(path), path.parent().map(fs::metadata)) {
        (Ok(own), Some(Ok(parent))) => own.dev() != parent.dev(),
        _ => false,
    }
}

/// An se mount whose connection is gone: listed, and `ENOTCONN` on access.
pub(crate) fn dead_mount(path: &Path) -> bool {
    let listed = canonical(path)
        .ok()
        .and_then(|canonical| super::fuse_abort::mount_at(&canonical))
        .is_some_and(|entry| entry.fstype == FUSE_TYPE);
    listed
        && fs::symlink_metadata(path)
            .err()
            .and_then(|error| error.raw_os_error())
            == Some(libc::ENOTCONN)
}

/// `fusermount3 -u [-z] -- <path>`. Without `lazy` a busy mount stays and
/// the error is `ResourceBusy`; a folder that is no mount is `NotFound`.
pub(crate) fn unmount(path: &Path, lazy: bool) -> io::Result<()> {
    let mut command = Command::new(super::fuse_runtime::helper_program());
    command.arg("-u");
    if lazy {
        command.arg("-z");
    }
    let output = command
        .arg("--")
        .arg(path)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let kind = if message.contains("busy") {
        io::ErrorKind::ResourceBusy
    } else if message.contains("not found")
        || message.contains("not mounted")
        || message.contains("Invalid argument")
    {
        io::ErrorKind::NotFound
    } else {
        io::ErrorKind::Other
    };
    Err(io::Error::new(kind, format!("fusermount3: {message}")))
}

fn set_mode(path: &Path, kind: PlaceKind, mode: u32) -> io::Result<()> {
    let mode = if kind == PlaceKind::Vault {
        mode | STICKY
    } else {
        mode
    };
    // Through a descriptor opened without following links: a folder that
    // was swapped for a link is never the target of the mode change.
    let directory: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    directory.set_permissions(fs::Permissions::from_mode(mode))
}

fn on_tmpfs(path: &Path) -> io::Result<bool> {
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| invalid("Einhängeort enthält NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    Ok(is_tmpfs_magic(stat.f_type))
}

/// `f_type` is a signed or unsigned word, depending on the C library.
fn is_tmpfs_magic(value: impl Into<i128>) -> bool {
    value.into() == TMPFS_MAGIC
}

fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn refused(path: &Path, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("Einhängeort {} {reason}", path.display()),
    )
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

#[cfg(test)]
#[path = "mount_point_tests.rs"]
mod tests;
