use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[path = "mountinfo.rs"]
mod mountinfo;
#[cfg(test)]
#[path = "review_task_local_guard_tests.rs"]
mod review_task_local_guard_tests;
#[cfg(test)]
#[path = "review_task_mountinfo_tests.rs"]
mod review_task_mountinfo_tests;
#[path = "stage.rs"]
mod stage;
#[path = "volume_id.rs"]
mod volume_id;

pub(crate) use stage::open_stage;

pub(crate) fn local_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

pub(crate) fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The OS path for a forward-slash VFS path; Unix names need no rewriting.
pub(crate) fn to_os(path: &str) -> PathBuf {
    PathBuf::from(path)
}

/// Key of the volume serving `path` for shared concurrency control: the
/// device of the nearest existing ancestor, so separate disks are separate.
pub(crate) fn volume_key(path: &str) -> String {
    use std::os::unix::fs::MetadataExt;
    let mut current = Path::new(path);
    loop {
        if let Ok(metadata) = std::fs::metadata(current) {
            return format!("local:dev{}", metadata.dev());
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return "local:".to_string(),
        }
    }
}

pub(crate) fn reported_name(path: &Path) -> Option<OsString> {
    path.file_name().map(OsString::from)
}

pub(crate) fn remove_file_like(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// `syncfs(2)` on the filesystem holding `path`: afterwards everything
/// written there before is on stable storage, as if each file had been
/// fsynced (Linux ≥ 2.6.39, bionic API 28).
pub(crate) fn flush_filesystem(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    // SAFETY: the descriptor belongs to `handle`, which outlives the call.
    if unsafe { libc::syncfs(handle.as_raw_fd()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Permission bits (with set-id and sticky bits) of a local entry.
pub(crate) fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o7777)
}

/// `fchmod` on an open file; FAT mounts refuse it, Android storage ignores it.
pub(crate) fn set_unix_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

/// Filesystem identity of `path` (filesystem UUID + location inside it).
/// `Ok(None)` = not determinable here, which callers treat as "unknown",
/// never as "another volume".
pub(crate) fn volume_identity(path: &Path) -> std::io::Result<Option<super::VolumeIdentity>> {
    volume_id::volume_identity(path)
}

/// Kind of filesystem mounted at the directory `path` when it is a mount
/// point inside its parent's tree (`None` = same filesystem as the parent).
/// Consult the mount table before touching the child: an autofs trigger
/// must be recognized without activating it. Bind mounts are boundaries
/// even when parent and child have the same device number.
pub(crate) fn mount_boundary(path: &Path) -> std::io::Result<Option<super::MountKind>> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(None);
    };
    let Some(name) = path.file_name() else {
        return Ok(None);
    };
    let candidate = std::fs::canonicalize(parent)?.join(name);
    let mounts = mountinfo::read()?;
    Ok(mountinfo::boundary(&mounts, &candidate))
}

/// What the filesystem holding `path` (or its nearest existing ancestor) can
/// store and how it flushes. Android's shared storage is a FUSE view: its
/// limits come from the filesystem below, its flushing stays FUSE's.
pub(crate) fn filesystem_profile(path: &Path) -> std::io::Result<super::fs_profile::FsProfile> {
    let resolved = mountinfo::resolve_existing(path)?;
    let mounts = mountinfo::read()?;
    let mount = mountinfo::containing(&mounts, &resolved);
    let fs_type = mount.map_or("", |mount| mount.fs_type.as_str());
    let mut profile = super::fs_profile::linux_profile(fs_type);
    if cfg!(target_os = "android") {
        if let Some(lower) = android_lower_type(&mounts, &resolved) {
            profile.limits = super::fs_profile::linux_profile(lower).limits;
        }
    } else if let Some(lower) = mount.and_then(volume_id::fuse_block_type) {
        profile.limits = super::fs_profile::linux_profile(&lower).limits;
    }
    Ok(profile)
}

/// Filesystem type below an Android storage view: `/data` for the built-in
/// storage, `/mnt/media_rw/<serial>` for SD cards and USB drives.
fn android_lower_type<'a>(mounts: &'a [mountinfo::Mount], path: &Path) -> Option<&'a str> {
    let volume = path.strip_prefix("/storage").ok()?.components().next()?;
    let lower = if volume.as_os_str() == "emulated" {
        PathBuf::from("/data/media")
    } else {
        Path::new("/mnt/media_rw").join(volume)
    };
    mountinfo::containing(mounts, &lower).map(|mount| mount.fs_type.as_str())
}

/// Device of a local entry, to tell which filesystem a stage is on.
pub(crate) fn device_of(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

/// A new file nobody else may read until it gets its final mode (`0600`).
pub(crate) fn create_new_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Unix names are bytes: every name the VFS hands over can be created.
pub(crate) fn check_new_name(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub(crate) fn fallback_limits() -> super::TargetLimits {
    super::TargetLimits::default()
}

/// Atomically replace `destination` (a regular file the caller checked) with
/// `source`.
pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

/// Publish without replacing: `renameat2(RENAME_NOREPLACE)`, then hard link +
/// unlink where the filesystem lacks the flag (NFS, sshfs, ntfs-3g), then a
/// checked rename for the app's own stages (FAT through FUSE, Android
/// storage) – the ladder in `android_fs`.
pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    crate::android_fs::rename_no_replace(source, destination)
}
