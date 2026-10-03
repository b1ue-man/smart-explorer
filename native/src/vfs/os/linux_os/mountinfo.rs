//! `/proc/self/mountinfo`: which mount holds a path, its filesystem type and
//! source, and where the mount's root lies inside its filesystem.
use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const MOUNTINFO: &str = "/proc/self/mountinfo";

/// One mount of the process's mount namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mount {
    /// `major:minor` of the filesystem (`st_dev` of its files).
    pub(crate) device: (u32, u32),
    /// Path of the mount's root inside its filesystem (`/`, a btrfs
    /// subvolume or the source folder of a bind mount).
    pub(crate) root: PathBuf,
    pub(crate) mount_point: PathBuf,
    pub(crate) fs_type: String,
    pub(crate) source: String,
}

pub(crate) fn read() -> io::Result<Vec<Mount>> {
    Ok(parse(&std::fs::read(MOUNTINFO)?))
}

/// The mounts of a mountinfo text; malformed lines are skipped.
pub(crate) fn parse(text: &[u8]) -> Vec<Mount> {
    text.split(|byte| *byte == b'\n')
        .filter_map(parse_line)
        .collect()
}

/// `id parent major:minor root mount-point options [optional…] - type source super-options`
fn parse_line(line: &[u8]) -> Option<Mount> {
    let fields: Vec<&[u8]> = line.split(|byte| *byte == b' ').collect();
    let separator = fields.iter().position(|field| *field == b"-")?;
    if separator < 6 || fields.len() < separator + 3 {
        return None;
    }
    let (major, minor) = std::str::from_utf8(fields[2]).ok()?.split_once(':')?;
    Some(Mount {
        device: (major.parse().ok()?, minor.parse().ok()?),
        root: PathBuf::from(OsString::from_vec(unescape(fields[3]))),
        mount_point: PathBuf::from(OsString::from_vec(unescape(fields[4]))),
        fs_type: String::from_utf8_lossy(&unescape(fields[separator + 1])).into_owned(),
        source: String::from_utf8_lossy(&unescape(fields[separator + 2])).into_owned(),
    })
}

/// Undoes the kernel's octal escapes (`\040` space, `\011` tab, `\012`
/// newline, `\134` backslash).
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] == b'\\' && index + 3 < field.len() {
            let digits = &field[index + 1..index + 4];
            if digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                let value = digits
                    .iter()
                    .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
                if let Ok(byte) = u8::try_from(value) {
                    out.push(byte);
                    index += 4;
                    continue;
                }
            }
        }
        out.push(field[index]);
        index += 1;
    }
    out
}

/// The mount holding the canonical `path`: the longest mount point that is
/// `path` or one of its ancestors; of equal ones the later line, which covers
/// the earlier.
pub(crate) fn containing<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    let mut best: Option<&Mount> = None;
    for mount in mounts {
        let longer = best.is_none_or(|current| {
            mount.mount_point.as_os_str().len() >= current.mount_point.as_os_str().len()
        });
        if longer && path.starts_with(&mount.mount_point) {
            best = Some(mount);
        }
    }
    best
}

/// Exact mount point, including same-device bind mounts and autofs. No
/// filesystem access to the child is needed to recognize a walk boundary.
pub(crate) fn boundary(mounts: &[Mount], path: &Path) -> Option<crate::vfs::MountKind> {
    mounts
        .iter()
        .rev()
        .find(|mount| mount.mount_point == path)
        .map(|mount| crate::vfs::fs_profile::mount_kind(&mount.fs_type))
}

/// `path` resolved through its nearest existing ancestor, with the missing
/// rest appended unchanged (a root that is created by the first run).
pub(crate) fn resolve_existing(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing = Vec::new();
    let mut current = absolute.as_path();
    loop {
        match std::fs::canonicalize(current) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (current.parent(), current.file_name()) else {
                    return Err(error);
                };
                missing.push(name.to_os_string());
                current = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// `major:minor` of a `st_dev`/`st_rdev` value (glibc, musl and bionic
/// encoding, `gnu_dev_major`/`gnu_dev_minor`).
pub(crate) fn split_device(device: u64) -> (u32, u32) {
    let major = ((device >> 32) & 0xffff_f000) | ((device >> 8) & 0x0000_0fff);
    let minor = ((device >> 12) & 0xffff_ff00) | (device & 0x0000_00ff);
    (major as u32, minor as u32)
}
