//! Negative image ownership needs kernel evidence, not a guess about a PID.
use super::{process_identity, File, OpenOptions, ProcessIdentity};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

/// A non-truncating writable descriptor excludes Linux exec of this inode.
/// Keep it through removal. Nothing is written and no process is signalled.
pub(in crate::mcp_launcher) fn removal_image_guard(path: &Path) -> Result<Option<File>, String> {
    let before = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("uninstall_image_status_unknown: {}: {error}", path.display())),
    };
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(format!("uninstall_image_not_regular: {}", path.display()));
    }
    let guard = OpenOptions::new().write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path).map_err(|error| format!("{}: {}: {error}",
            if error.raw_os_error() == Some(libc::ETXTBSY) {
                "uninstall_busy"
            } else {
                "uninstall_image_exclusion_unavailable"
            }, path.display()))?;
    let held = guard.metadata().map_err(|error| error.to_string())?;
    let after = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !held.is_file() || !after.is_file() || after.file_type().is_symlink()
        || before.dev() != held.dev() || before.ino() != held.ino()
        || after.dev() != held.dev() || after.ino() != held.ino()
    {
        return Err("uninstall_image_object_changed".into());
    }
    Ok(Some(guard))
}

pub(in crate::mcp_launcher) fn processes_for(path: &Path) -> Result<Vec<ProcessIdentity>, String> {
    processes_for_in(path, Path::new("/proc"))
}

fn processes_for_in(path: &Path, proc_root: &Path) -> Result<Vec<ProcessIdentity>, String> {
    match removal_image_guard(path) {
        // The kernel excludes execution of this exact inode even if /proc
        // hides an unrelated user's executable. This is not a UID exemption.
        Ok(Some(_guard)) => return Ok(Vec::new()),
        Ok(None) => return Ok(Vec::new()), // no named object can be removed
        Err(_) => {} // no proof: retain the conservative process inventory
    }
    let target = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !target.is_file() || target.file_type().is_symlink() {
        return Err("runtime_process_image_not_regular".into());
    }
    let mut result = Vec::new();
    for entry in std::fs::read_dir(proc_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        // Tests inject only the proc directory inventory, never a production
        // environment override or a replacement process-identity provider.
        match process_identity(pid) {
            Ok(identity) if Path::new(&identity.image) == path
                || identity.executable_object.as_ref().is_some_and(|object|
                    object.device == target.dev() && object.inode == target.ino()) => result.push(identity),
            Ok(_) => {}
            Err(_) => {
                let metadata = match std::fs::metadata(entry.path()) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.to_string()),
                    Ok(metadata) => metadata,
                };
                if let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) {
                    if let Some((_, tail)) = stat.rsplit_once(") ") {
                        let mut fields = tail.split_whitespace();
                        let state = fields.next();
                        let flags = fields.nth(5).and_then(|s| s.parse::<u64>().ok());
                        if state == Some("Z") || flags.is_some_and(|flags| flags & 0x0020_0000 != 0) {
                            continue;
                        }
                    }
                }
                return Err(if metadata.uid() == unsafe { libc::geteuid() } {
                    "runtime_process_status_unknown"
                } else {
                    "runtime_cross_user_process_status_unknown"
                }.into());
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_p13_linux_idle_image_proof_preserves_bytes_despite_unknown_processes() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("se");
        std::fs::write(&image, b"\x7fELF-fixture-image").unwrap();
        let before = std::fs::metadata(&image).unwrap();
        let unknown_proc = root.path().join("proc");
        std::fs::create_dir(&unknown_proc).unwrap();
        std::fs::create_dir(unknown_proc.join(u32::MAX.to_string())).unwrap();
        assert!(processes_for_in(&image, &unknown_proc).unwrap().is_empty());
        let guard = removal_image_guard(&image).unwrap().unwrap();
        assert_eq!(std::fs::read(&image).unwrap(), b"\x7fELF-fixture-image");
        assert_eq!(std::fs::metadata(&image).unwrap().modified().unwrap(), before.modified().unwrap());
        std::fs::remove_file(&image).unwrap();
        assert_eq!(guard.metadata().unwrap().ino(), before.ino());
    }

    #[test]
    fn batches_p13_linux_running_image_and_unknown_owners_remain_blocked() {
        let image = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert!(removal_image_guard(&image).unwrap_err().starts_with("uninstall_busy:"));
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(u32::MAX.to_string())).unwrap();
        assert_eq!(processes_for_in(&image, root.path()).unwrap_err(), "runtime_process_status_unknown");
    }

    #[test]
    fn batches_p13_linux_image_exclusion_never_follows_a_foreign_link() {
        let root = tempfile::tempdir().unwrap();
        let foreign = root.path().join("foreign");
        std::fs::write(&foreign, b"keep").unwrap();
        let link = root.path().join("se");
        std::os::unix::fs::symlink(&foreign, &link).unwrap();
        assert!(removal_image_guard(&link).unwrap_err().starts_with("uninstall_image_not_regular:"));
        assert_eq!(std::fs::read(foreign).unwrap(), b"keep");
    }
}
