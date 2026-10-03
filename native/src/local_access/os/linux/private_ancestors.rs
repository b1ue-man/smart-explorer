//! Identity ancestry, including bind aliases whose namespace parent differs.
use super::DirectoryHandle;
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Path, PathBuf},
};

impl DirectoryHandle {
    pub(crate) fn is_within_any(&self, roots: &[PathBuf]) -> io::Result<bool> {
        let mut private = Vec::new();
        for path in roots {
            match Self::open_root(path) {
                Ok(root) => private.push(root),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if private.is_empty() {
            return Ok(false);
        }
        let identities = private
            .iter()
            .map(|root| root.metadata().map(|m| (m.dev(), m.ino())))
            .collect::<io::Result<Vec<_>>>()?;
        let mut current = self.clone();
        let mut reached_root = false;
        for _ in 0..4096 {
            let metadata = current.metadata()?;
            let identity = (metadata.dev(), metadata.ino());
            if identities.contains(&identity) {
                return Ok(true);
            }
            let parent_file = current.open_at(
                OsStr::new(".."),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )?;
            let parent_metadata = parent_file.metadata()?;
            if (parent_metadata.dev(), parent_metadata.ino()) == identity {
                reached_root = true;
                break;
            }
            current = Self {
                file: std::sync::Arc::new(parent_file),
                path: PathBuf::new(),
            };
        }
        if !reached_root {
            return Err(unknown());
        }
        let device = self.metadata()?.dev();
        let same_device = private
            .iter()
            .zip(&identities)
            .filter(|(_, id)| id.0 == device);
        let mut location = None;
        for (root, _) in same_device {
            // A bind mount of a private subdirectory can have a public '..'.
            // Compare the fd's mount-root-relative physical location as well.
            if location.is_none() {
                location = Some(filesystem_location(&self.file)?);
            }
            let candidate = location.as_ref().ok_or_else(unknown)?;
            if candidate.starts_with(filesystem_location(&root.file)?) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn filesystem_location(file: &File) -> io::Result<PathBuf> {
    let fd = file.as_raw_fd();
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}"))?;
    let mount = info
        .lines()
        .find_map(|line| {
            line.strip_prefix("mnt_id:")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .ok_or_else(unknown)?;
    let spelling = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
    let table = std::fs::read_to_string("/proc/self/mountinfo")?;
    for line in table.lines() {
        let mut fields = line.split_ascii_whitespace();
        if fields.next().and_then(|value| value.parse::<u64>().ok()) != Some(mount) {
            continue;
        }
        let _parent = fields.next();
        let _device = fields.next();
        let root = decode_mount_path(fields.next().ok_or_else(unknown)?)?;
        let point = decode_mount_path(fields.next().ok_or_else(unknown)?)?;
        let relative = spelling.strip_prefix(&point).map_err(|_| unknown())?;
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(unknown());
        }
        return Ok(root.join(relative));
    }
    Err(unknown())
}

fn decode_mount_path(encoded: &str) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let digits = bytes.get(index + 1..index + 4).ok_or_else(unknown)?;
        if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
            return Err(unknown());
        }
        let value = (digits[0] - b'0') as u16 * 64
            + (digits[1] - b'0') as u16 * 8
            + (digits[2] - b'0') as u16;
        decoded.push(u8::try_from(value).map_err(|_| unknown())?);
        index += 4;
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(decoded));
    if !Path::new(&path).is_absolute() {
        return Err(unknown());
    }
    Ok(path)
}
fn unknown() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "private-root physical ancestry could not be established",
    )
}
