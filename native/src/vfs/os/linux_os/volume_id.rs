//! Filesystem identity of a local Linux/Android location, independent of the
//! mount point: the filesystem UUID (udev `by-uuid` links, else the udev
//! database) or the source of a network/FUSE mount, plus the location inside
//! the filesystem.
use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::mountinfo::{self, Mount};
use crate::vfs::fs_profile::mount_kind;
use crate::vfs::{MountKind, VolumeIdentity};

const BY_UUID: &str = "/dev/disk/by-uuid";
const UDEV_DATA: &str = "/run/udev/data";

pub(crate) fn volume_identity(path: &Path) -> io::Result<Option<VolumeIdentity>> {
    let canonical = mountinfo::resolve_existing(path)?;
    if cfg!(target_os = "android") {
        if let Some(identity) = android_storage_identity(&canonical) {
            return Ok(Some(identity));
        }
    }
    let mounts = mountinfo::read()?;
    let Some(mount) = mountinfo::containing(&mounts, &canonical) else {
        return Ok(None);
    };
    let Some(volume_id) = filesystem_uuid(mount).or_else(|| network_source(mount)) else {
        return Ok(None);
    };
    let inside = canonical
        .strip_prefix(&mount.mount_point)
        .unwrap_or_else(|_| Path::new(""));
    Ok(Some(VolumeIdentity {
        volume_id,
        relative_path: forward(&mount.root.join(inside)),
        fs_type: mount.fs_type.clone(),
    }))
}

/// Android names its volumes in the path: `/storage/emulated/<user>/…` for
/// the built-in storage, `/storage/<FAT/exFAT serial>/…` for SD cards and USB
/// drives (vold mounts them by filesystem UUID).
fn android_storage_identity(canonical: &Path) -> Option<VolumeIdentity> {
    let mut components = canonical.strip_prefix("/storage").ok()?.components();
    let volume = components.next()?.as_os_str().to_str()?;
    if volume == "self" {
        return None;
    }
    let inside: PathBuf = components.collect();
    Some(VolumeIdentity {
        volume_id: volume.to_ascii_lowercase(),
        relative_path: forward(&inside),
        fs_type: "android-storage".into(),
    })
}

fn filesystem_uuid(mount: &Mount) -> Option<String> {
    let mut devices = vec![mount.device];
    if mount.source.starts_with("/dev/") {
        if let Ok(metadata) = std::fs::metadata(&mount.source) {
            devices.push(mountinfo::split_device(metadata.rdev()));
        }
    }
    uuid_from_links(&devices).or_else(|| devices.iter().find_map(|device| uuid_from_udev(*device)))
}

/// The `by-uuid` link whose device node is one of `devices`.
fn uuid_from_links(devices: &[(u32, u32)]) -> Option<String> {
    for entry in std::fs::read_dir(BY_UUID).ok()?.flatten() {
        let Ok(metadata) = std::fs::metadata(entry.path()) else {
            continue;
        };
        if devices.contains(&mountinfo::split_device(metadata.rdev())) {
            return Some(decode_udev_name(&entry.file_name()));
        }
    }
    None
}

/// `E:ID_FS_UUID=` of the udev database entry of a block device.
fn uuid_from_udev((major, minor): (u32, u32)) -> Option<String> {
    if major == 0 {
        return None;
    }
    let text = std::fs::read_to_string(format!("{UDEV_DATA}/b{major}:{minor}")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("E:ID_FS_UUID="))
        .filter(|uuid| !uuid.is_empty())
        .map(str::to_ascii_lowercase)
}

/// udev link names encode special characters as `\xHH`.
fn decode_udev_name(name: &OsStr) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = bytes
            .get(index..index + 4)
            .filter(|chunk| chunk.starts_with(b"\\x"))
            .and_then(|chunk| std::str::from_utf8(&chunk[2..]).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

/// A network or FUSE mount is identified by what it mounts (`server:/export`,
/// `//server/share`, `user@host:/path`), never by its anonymous device.
fn network_source(mount: &Mount) -> Option<String> {
    let remote = matches!(
        mount_kind(&mount.fs_type),
        MountKind::Network | MountKind::Fuse
    );
    let named = !mount.source.is_empty() && mount.source != "none";
    (remote && named && !mount.source.starts_with("/dev/"))
        .then(|| format!("{}:{}", mount.fs_type, mount.source))
}

/// The location as forward-slash text without a leading slash.
fn forward(path: &Path) -> String {
    path.to_string_lossy().trim_start_matches('/').to_string()
}
