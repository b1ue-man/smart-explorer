use super::super::core::lifecycle::{ProcessIdentity, StopIdentity};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
#[path = "spawn.rs"]
mod spawn;
#[path = "process_inventory.rs"]
mod process_inventory;
#[path = "retirement.rs"]
mod retirement;
pub(in crate::mcp_launcher) use process_inventory::{processes_for, removal_image_guard};
pub(in crate::mcp_launcher) use retirement::{retirement_handoff, retirement_root};
pub(in crate::mcp_launcher) use spawn::*;

pub(in crate::mcp_launcher) const CLI_NAME: &str = "se";
pub(in crate::mcp_launcher) const LAUNCHER_NAME: &str = "se-mcp-launcher";
#[cfg(test)]
pub(in crate::mcp_launcher) fn create_test_directory_link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}
pub(in crate::mcp_launcher) fn uninstall_programs_idle(root: &Path) -> Result<Vec<File>, String> {
    let mut guards = Vec::new();
    for name in [
        CLI_NAME,
        "smart_explorer",
        "Smart Explorer",
        "smart_explorer_updater",
        "Smart Explorer Updater",
    ] {
        if let Some(guard) = removal_image_guard(&root.join(name))? {
            guards.push(guard);
        }
    }
    Ok(guards)
}

pub(in crate::mcp_launcher) fn validate_tree(root: &Path, path: &Path) -> Result<(), String> {
    if !root.is_absolute() || !path.starts_with(root) {
        return Err("runtime_path_outside_installation".into());
    }
    for ancestor in root.ancestors().skip(1) {
        let meta = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || (meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0)
        {
            return Err(format!(
                "runtime_ancestor_writable_or_link: {}",
                ancestor.display()
            ));
        }
    }
    let owner = std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .uid();
    if owner != 0 && owner != unsafe { libc::geteuid() } {
        return Err("runtime_installation_owner_invalid".into());
    }
    let mut current = root.to_path_buf();
    check_node(&current, owner)?;
    for component in path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            return Err("runtime_path_component_invalid".into());
        };
        current.push(name);
        check_node(&current, owner)?;
    }
    let root_c =
        std::ffi::CString::new(root.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(root_c.as_ptr(), &mut stat) } != 0 {
        return Err(format!(
            "runtime_filesystem_check: {}",
            std::io::Error::last_os_error()
        ));
    }
    // NFS, SMB/CIFS and FUSE/network mounts have no admitted lock contract.
    if matches!(
        stat.f_type as u64,
        0x6969 | 0x517b | 0xff534d42 | 0x65735546
    ) {
        return Err("runtime_installation_lock_filesystem_unsupported".into());
    }
    Ok(())
}
fn check_node(path: &Path, owner: u32) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink()
        || (!meta.is_file() && !meta.is_dir())
        || meta.uid() != owner
        || meta.mode() & 0o022 != 0
    {
        return Err(format!(
            "runtime_permissions_or_link_invalid: {}",
            path.display()
        ));
    }
    Ok(())
}
pub(in crate::mcp_launcher) fn open_verified(
    root: &Path,
    path: &Path,
    hash: &str,
) -> Result<File, String> {
    validate_tree(root, path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err("runtime_native_executable_required".into());
    }
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if magic != *b"\x7fELF" {
        return Err("runtime_native_elf_required".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    super::super::installation::verify_handle(&mut file, hash)?;
    Ok(file)
}
pub(in crate::mcp_launcher) fn lock(path: &Path, exclusive: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| format!("lock {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("lock_not_regular".into());
    }
    let mode = (if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    }) | libc::LOCK_NB;
    if unsafe { libc::flock(file.as_raw_fd(), mode) } != 0 {
        let error = std::io::Error::last_os_error();
        return Err(format!(
            "{}: {}: {error}",
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                "lock_busy"
            } else {
                "lock_denied"
            },
            path.display()
        ));
    }
    Ok(file)
}
pub(in crate::mcp_launcher) fn legacy_profile_image(path: &Path) -> Result<File, String> {
    lock(path, false)
}
pub(in crate::mcp_launcher) fn legacy_update_image(path: &Path) -> Result<Option<File>, String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    lock(path, true).map(Some)
}
pub(in crate::mcp_launcher) fn same_image_file(file: &File, path: &Path) -> Result<bool, String> {
    let held = file.metadata().map_err(|e| e.to_string())?;
    let named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    Ok(held.is_file() && named.is_file() && held.dev() == named.dev() && held.ino() == named.ino())
}
pub(in crate::mcp_launcher) fn protect(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() || executable {
        0o755
    } else {
        0o644
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
}
pub(in crate::mcp_launcher) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
pub(in crate::mcp_launcher) fn process_identity(pid: u32) -> Result<ProcessIdentity, String> {
    let base = format!("/proc/{pid}");
    let stat = std::fs::read_to_string(format!("{base}/stat")).map_err(|e| e.to_string())?;
    let (_, tail) = stat.rsplit_once(") ").ok_or("process_stat_invalid")?;
    let started = tail
        .split_whitespace()
        .nth(19)
        .ok_or("process_start_missing")?;
    // Opening the proc link yields the actual mapped executable, including
    // after unlink/rename. Never strip the diagnostic " (deleted)" suffix.
    let executable = File::open(format!("{base}/exe")).map_err(|e| e.to_string())?;
    let metadata = executable.metadata().map_err(|e| e.to_string())?;
    let object = super::super::core::lifecycle::ExecutableObject {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let image = std::fs::read_link(format!("{base}/exe")).map_err(|e| e.to_string())?;
    let after = std::fs::read_to_string(format!("{base}/stat")).map_err(|e| e.to_string())?;
    if after
        .rsplit_once(") ")
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        != Some(started)
    {
        return Err("runtime_process_identity_changed".into());
    }
    Ok(ProcessIdentity {
        pid,
        started: started.into(),
        image: image.to_string_lossy().into_owned(),
        executable_object: Some(object),
    })
}
pub(in crate::mcp_launcher) fn stop_identity(path: &Path) -> Result<StopIdentity, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("stop_marker_invalid".into());
    }
    Ok(StopIdentity {
        file_id: format!("{}:{}", meta.dev(), meta.ino()),
        modified: format!("{}:{}:{}", meta.mtime(), meta.mtime_nsec(), meta.len()),
        sha256: super::super::installation::hash_bytes(
            &std::fs::read(path).map_err(|e| e.to_string())?,
        ),
    })
}

pub(in crate::mcp_launcher) fn process_matches(expected: &ProcessIdentity) -> bool {
    if let Ok(actual) = process_identity(expected.pid) {
        return actual.pid == expected.pid
            && actual.started == expected.started
            && match (&expected.executable_object, &actual.executable_object) {
                (Some(expected), Some(actual)) => expected == actual,
                _ => false,
            };
    }
    // A protected owner record + occupied CLOEXEC owner lock + unchanged
    // kernel starttime still identify the updater when hidepid denies exe.
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", expected.pid)) else {
        return false;
    };
    let Some((_, tail)) = stat.rsplit_once(") ") else {
        return false;
    };
    tail.split_whitespace().nth(19) == Some(expected.started.as_str())
}

pub(in crate::mcp_launcher) fn publish_staged(
    staged: tempfile::NamedTempFile,
    target: &Path,
    replace: bool,
) -> Result<(), String> {
    if replace {
        staged.persist(target).map_err(|e| e.to_string())?;
    } else {
        staged
            .persist_noclobber(target)
            .map_err(|e| e.to_string())?;
    }
    sync_directory(target.parent().ok_or("runtime_record_parent_missing")?)
}
pub(in crate::mcp_launcher) fn create_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|e| e.to_string())?;
    protect(path, true)?;
    sync_directory(path.parent().ok_or("runtime_directory_parent_missing")?)
}
pub(in crate::mcp_launcher) fn rename_durable(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(source, target).map_err(|e| e.to_string())?;
    sync_directory(target.parent().ok_or("runtime_record_parent_missing")?)
}
pub(in crate::mcp_launcher) fn running_image() -> Result<File, String> {
    File::open("/proc/self/exe").map_err(|e| e.to_string())
}
