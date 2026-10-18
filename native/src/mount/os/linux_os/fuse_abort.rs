//! The kernel's view of a FUSE drive (DRV-M4, decision C2 "Abbruch").
//!
//! `/proc/self/mountinfo` says whether a folder is a mount point, of which
//! type, and through field 3 (`major:minor`) which FUSE connection serves
//! it. `se drive unmount --force` and the vault lock deadline (DRV-M7) end a
//! connection that hangs in a backend by writing to
//! `/sys/fs/fuse/connections/<n>/abort`; `<n>` is the minor number of the
//! mount (assumption A5 of strand S2: FUSE mounts have major 0, and the
//! directory name is `fc->dev`). The number is read from `mountinfo`, never
//! by `stat` on the hanging mount point. The connection files belong to the
//! user who mounted, so no privilege is needed; without `fusectl` only the
//! lazy unmount remains.
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const MOUNTINFO: &str = "/proc/self/mountinfo";
const CONNECTIONS: &str = "/sys/fs/fuse/connections";

/// One line of `mountinfo`, as far as the host needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MountEntry {
    pub(crate) mount_point: PathBuf,
    pub(crate) fstype: String,
    pub(crate) major: u32,
    pub(crate) minor: u32,
}

/// Every parsable line of a `mountinfo` text, in kernel order.
pub(crate) fn parse_mountinfo(text: &str) -> Vec<MountEntry> {
    text.lines().filter_map(parse_line).collect()
}

/// The top-most mount at `path` (the last entry for it), if any.
pub(crate) fn mount_at(path: &Path) -> Option<MountEntry> {
    let text = fs::read_to_string(MOUNTINFO).ok()?;
    mount_in(&text, path)
}

pub(crate) fn mount_in(text: &str, path: &Path) -> Option<MountEntry> {
    parse_mountinfo(text)
        .into_iter()
        .rev()
        .find(|entry| entry.mount_point == path)
}

/// The FUSE connection number of an se drive at `path`.
pub(crate) fn connection_at(path: &Path) -> Option<u32> {
    mount_at(path)
        .filter(|entry| entry.fstype == super::mount_point::FUSE_TYPE)
        .map(|entry| connection_number(&entry))
}

/// The kernel names a connection after the device of the mount
/// (`fc->dev`, `MINORBITS` = 20); a FUSE mount has major 0, so the name is
/// its minor number (assumption A5).
pub(crate) fn connection_number(entry: &MountEntry) -> u32 {
    entry.major.wrapping_shl(20) | entry.minor
}

/// Ends the FUSE connection `connection`: every waiting request returns
/// `ENOTCONN`/`ECONNABORTED`, the serving process sees `ENODEV`.
pub(crate) fn abort_connection(connection: u32) -> io::Result<()> {
    abort_in(Path::new(CONNECTIONS), connection)
}

pub(crate) fn abort_in(connections: &Path, connection: u32) -> io::Result<()> {
    let abort = connections.join(connection.to_string()).join("abort");
    if !abort.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} fehlt (fusectl ist nicht eingehängt); nur das verzögerte Aushängen bleibt",
                abort.display()
            ),
        ));
    }
    fs::write(&abort, b"1")
}

fn parse_line(line: &str) -> Option<MountEntry> {
    let (before, after) = line.split_once(" - ")?;
    let fields: Vec<&str> = before.split(' ').collect();
    let device = fields.get(2)?;
    let (major, minor) = device.split_once(':')?;
    let mount_point = unescape(fields.get(4)?)?;
    let fstype = after.split(' ').next()?.to_string();
    Some(MountEntry {
        mount_point,
        fstype,
        major: major.parse().ok()?,
        minor: minor.parse().ok()?,
    })
}

/// `mountinfo` writes space, tab, newline and backslash as `\ooo`.
fn unescape(field: &str) -> Option<PathBuf> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let digits = bytes.get(index + 1..index + 4)?;
            if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                return None;
            }
            let value = digits
                .iter()
                .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
            out.push(u8::try_from(value).ok()?);
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(out)))
}
