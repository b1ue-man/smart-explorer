//! Post-publication namespace confirmation on the opened parent directory.
//! Android's MediaProvider FUSE is qualified separately from generic FUSE.
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use super::mountinfo::{self, Mount};
use super::super::fs_profile::{linux_profile, FlushModel};
use crate::local_access::DirectoryHandle;

pub(crate) fn confirm_namespace(parent: &Path) -> io::Result<bool> {
    let directory = DirectoryHandle::open_root(parent)?;
    let file = directory.directory_file();
    // Resolve only this live FD, keeping root aliases and the actual opened
    // object tied together even if a free path changes during the call.
    let anchor = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    let resolved = std::fs::canonicalize(anchor)?;
    let mounts = mountinfo::read()?;
    let Some(mount) = mountinfo::containing(&mounts, &resolved) else {
        return Ok(false);
    };
    let metadata = file.metadata()?;
    #[allow(clippy::unnecessary_cast)] // Android's helpers return c_int.
    let device = (
        libc::major(metadata.dev() as libc::dev_t) as u32,
        libc::minor(metadata.dev() as libc::dev_t) as u32,
    );
    if mount.device != device {
        return Ok(false);
    }
    let ordinary = linux_profile(&mount.fs_type).flush != FlushModel::PerFileOnly;
    if !ordinary && !media_provider_mount(&mounts, mount) {
        return Ok(false);
    }
    // The file is an O_RDONLY directory pin, never O_PATH or a data file.
    // Unsupported/error stays unconfirmed; do not substitute file fsync.
    file.sync_all()?;
    Ok(true)
}

fn media_provider_mount(mounts: &[Mount], selected: &Mount) -> bool {
    cfg!(target_os = "android")
        && selected.fs_type == "fuse"
        && selected.source == "/dev/fuse"
        && mounts.iter().any(|mount| {
            mount.device == selected.device
                && mount.fs_type == "fuse"
                && mount.source == "/dev/fuse"
                && system_storage_root(&mount.mount_point, &mount.root)
        })
}

/// Require a vold storage root of the same device, allowing its known bind
/// aliases without treating a directory merely named /storage as proof.
fn system_storage_root(point: &Path, root: &Path) -> bool {
    let Some(point) = point.to_str() else {
        return false;
    };
    let view = if let Some(view) = point.strip_prefix("/storage/") {
        view
    } else if let Some(view) = point.strip_prefix("/mnt/user/") {
        let Some((user, view)) = view.split_once('/') else {
            return false;
        };
        if !decimal(user) {
            return false;
        }
        view
    } else {
        return false;
    };
    let (label, user) = match view.split_once('/') {
        Some((label, user)) => (label, Some(user)),
        None => (view, None),
    };
    if !volume_label(label) {
        return false;
    }
    match user {
        Some(user) => decimal(user) && root == Path::new(&format!("/{user}")),
        None => root == Path::new("/"),
    }
}

fn decimal(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u32>().is_ok()
}

fn volume_label(label: &str) -> bool {
    if label == "emulated" {
        return true;
    }
    if let Some(device) = label.strip_prefix("public:") {
        return device
            .split_once(',')
            .is_some_and(|(major, minor)| decimal(major) && decimal(minor));
    }
    let separators: &[usize] = match label.len() {
        9 => &[4],
        36 => &[8, 13, 18, 23],
        _ => return false,
    };
    label.bytes().enumerate().all(|(index, byte)| {
        if separators.contains(&index) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    })
}
