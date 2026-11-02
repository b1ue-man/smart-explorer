//! Same-process retirement handoff, bound to the image's own installation.
use crate::mcp_launcher::installation::{handle_sha256, verify_handle, RetirementRoot};
use std::fs::{File, OpenOptions};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const ROOT_FD_ENV: &str = "SE_MCP_RETIREMENT_ROOT_FD";
pub(super) const SNAPSHOT_ENV: &str = "SE_MCP_RETIREMENT_SNAPSHOT_SHA256";
const PREFIX: &str = ".mcp-retire-";

/// The descriptor is only corroborating evidence. The root is derived from
/// the actual running stage path, never selected by an environment value.
pub(in crate::mcp_launcher) fn retirement_root() -> Result<Option<RetirementRoot>, String> {
    let Some(raw) = std::env::var_os(ROOT_FD_ENV) else { return Ok(None); };
    std::env::remove_var(ROOT_FD_ENV);
    let fd = raw.to_str().and_then(|s| s.parse::<i32>().ok())
        .filter(|fd| *fd > 2).ok_or("uninstall_handoff_descriptor_invalid")?;
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err("uninstall_handoff_descriptor_invalid".into());
    }
    let root_pin = unsafe { File::from_raw_fd(fd) };
    let snapshot = std::env::var(SNAPSHOT_ENV).map_err(|_| "uninstall_handoff_snapshot_missing")?;
    std::env::remove_var(SNAPSHOT_ENV);
    if !crate::mcp_launcher::core::manifest::is_sha256(&snapshot) {
        return Err("uninstall_handoff_snapshot_invalid".into());
    }
    let image = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let stage = image.parent().ok_or("uninstall_handoff_stage_missing")?;
    let root = stage.parent().ok_or("uninstall_handoff_root_missing")?;
    let id = stage.file_name().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix(PREFIX));
    if image.file_name() != Some(std::ffi::OsStr::new(super::CLI_NAME))
        || !id.is_some_and(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("uninstall_handoff_stage_invalid".into());
    }
    super::validate_tree(root, &image)?;
    let pinned = root_pin.metadata().map_err(|e| e.to_string())?;
    let named = std::fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    let stage_metadata = std::fs::symlink_metadata(stage).map_err(|e| e.to_string())?;
    if !pinned.is_dir() || pinned.dev() != named.dev() || pinned.ino() != named.ino()
        || stage_metadata.uid() != unsafe { libc::geteuid() }
        || stage_metadata.mode() & 0o077 != 0
    {
        return Err("uninstall_handoff_root_or_owner_changed".into());
    }
    let mut running = super::running_image()?;
    if !super::same_image_file(&running, &image)? {
        return Err("uninstall_handoff_running_image_changed".into());
    }
    let hash = handle_sha256(&mut running)?;
    let _installed = super::open_verified(root, &root.join(super::CLI_NAME), &hash)?;
    // The shared caller rechecks the same manifest/removal hash before touching
    // MCP files. Clean only our bound image and then the empty private directory.
    let stage = OwnStage { directory: stage.into(), image: image.clone(),
        directory_object: (stage_metadata.dev(), stage_metadata.ino()),
        image_object: Some(object(&running.metadata().map_err(|e| e.to_string())?)),
    };
    stage.cleanup()?;
    Ok(Some(RetirementRoot { root: root.into(), snapshot_sha256: snapshot }))
}

/// Stage on the already executable installation filesystem, so noexec /tmp
/// and unrelated unwritable directories cannot obstruct the handoff.
pub(in crate::mcp_launcher) fn retirement_handoff(root: &Path, expected: &str, snapshot: &str) -> Result<(), String> {
    super::validate_tree(root, root)?;
    let mut source = super::running_image()?;
    verify_handle(&mut source, expected)?;
    if !super::same_image_file(&source, &root.join(super::CLI_NAME))? {
        return Err("uninstall_running_installation_changed".into());
    }
    let directory = root.join(format!("{PREFIX}{}", crate::mcp_launcher::provision::unique_id()?));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)
        .map_err(|e| format!("uninstall_handoff_stage_create: {}: {e}", directory.display()))?;
    let metadata = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
    let mut stage = OwnStage { image: directory.join(super::CLI_NAME), directory,
        directory_object: object(&metadata), image_object: None };
    let mut output = OpenOptions::new().read(true).write(true).create_new(true).mode(0o700)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW).open(&stage.image)
        .map_err(|e| e.to_string())?;
    stage.image_object = Some(object(&output.metadata().map_err(|e| e.to_string())?));
    std::io::copy(&mut source, &mut output).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    verify_handle(&mut output, expected)?;
    drop(output); // A writable stage handle would itself forbid fexecve.
    let image = super::open_verified(root, &stage.image, expected)?;
    let root_pin = OpenOptions::new().read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root).map_err(|e| e.to_string())?;
    super::spawn::replace_for_retirement(&stage.image, image, root_pin, expected, snapshot)
}

fn object(metadata: &std::fs::Metadata) -> (u64, u64) { (metadata.dev(), metadata.ino()) }

struct OwnStage {
    directory: PathBuf,
    image: PathBuf,
    directory_object: (u64, u64),
    image_object: Option<(u64, u64)>,
}
impl OwnStage {
    fn cleanup(&self) -> Result<(), String> {
        let directory = match std::fs::symlink_metadata(&self.directory) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()), Ok(metadata) => metadata,
        };
        if !directory.is_dir() || directory.file_type().is_symlink()
            || object(&directory) != self.directory_object {
            return Err("uninstall_handoff_stage_replaced".into());
        }
        match std::fs::symlink_metadata(&self.image) {
            Ok(image) if image.is_file() && !image.file_type().is_symlink()
                && self.image_object == Some(object(&image)) => {
                std::fs::remove_file(&self.image).map_err(|e| e.to_string())?;
            }
            Ok(_) => return Err("uninstall_handoff_stage_image_replaced".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        std::fs::remove_dir(&self.directory)
            .map_err(|e| format!("uninstall_handoff_stage_cleanup: {}: {e}", self.directory.display()))
    }
}
impl Drop for OwnStage {
    fn drop(&mut self) { let _ = self.cleanup(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    #[test]
    fn batches_p13_linux_retirement_cleanup_preserves_foreign_stage_entries() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("stage");
        std::fs::create_dir(&directory).unwrap();
        let image = directory.join("se");
        std::fs::write(&image, b"owned").unwrap();
        let stage = OwnStage { directory: directory.clone(), image: image.clone(),
            directory_object: object(&std::fs::symlink_metadata(&directory).unwrap()),
            image_object: Some(object(&std::fs::symlink_metadata(&image).unwrap())) };
        let foreign = directory.join("user-note");
        std::fs::write(&foreign, b"keep").unwrap();
        assert!(stage.cleanup().unwrap_err().contains("uninstall_handoff_stage_cleanup:"));
        assert!(!image.exists());
        assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
        let replacement = directory.join("replacement");
        std::fs::write(&replacement, b"new user image").unwrap();
        std::fs::rename(&replacement, &image).unwrap();
        assert_eq!(stage.cleanup().unwrap_err(), "uninstall_handoff_stage_image_replaced");
        assert_eq!(std::fs::read(&image).unwrap(), b"new user image");
    }

    #[test]
    fn batches_p13_linux_retirement_handoff_cannot_select_a_foreign_installation() {
        use std::os::unix::process::CommandExt;
        let source = std::env::var_os("SE_SCREEN_TASK_WORKER")
            .expect("the existing remote runner must provide the candidate se binary");
        let source = std::fs::canonicalize(source).unwrap();
        let own = tempfile::tempdir_in(source.parent().unwrap()).unwrap();
        let root = std::fs::canonicalize(own.path()).unwrap();
        let directory = root.join(format!("{PREFIX}{}", "a".repeat(32)));
        std::fs::DirBuilder::new().mode(0o700).create(&directory).unwrap();
        let image = directory.join(super::super::CLI_NAME);
        std::fs::copy(&source, &image).unwrap();
        std::fs::copy(&source, root.join(super::super::CLI_NAME)).unwrap();
        let foreign = tempfile::tempdir_in(source.parent().unwrap()).unwrap();
        let keep = foreign.path().join("keep.txt");
        std::fs::write(&keep, b"foreign installation data").unwrap();
        let foreign_pin = File::open(foreign.path()).unwrap();
        let foreign_fd = foreign_pin.as_raw_fd();
        let mut command = std::process::Command::new(&image);
        command.args(["plugin", "retire-installation"])
            .env(ROOT_FD_ENV, foreign_fd.to_string()).env(SNAPSHOT_ENV, "0".repeat(64))
            .env_remove("COMPLETE");
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(foreign_fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("uninstall_handoff_root_or_owner_changed"));
        assert!(image.exists(), "failed foreign handoff must not clean an unproved stage");
        assert!(root.join(super::super::CLI_NAME).exists());
        assert_eq!(std::fs::read(keep).unwrap(), b"foreign installation data");
        // A genuine root descriptor is still insufficient when the caller's
        // validated installation/removal snapshot no longer agrees.
        let own_pin = File::open(&root).unwrap();
        let own_fd = own_pin.as_raw_fd();
        let mut changed = std::process::Command::new(&image);
        changed.args(["plugin", "retire-installation"])
            .env(ROOT_FD_ENV, own_fd.to_string()).env(SNAPSHOT_ENV, "0".repeat(64))
            .env_remove("COMPLETE");
        unsafe {
            changed.pre_exec(move || {
                if libc::fcntl(own_fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let changed = changed.output().unwrap();
        assert!(!changed.status.success());
        assert!(String::from_utf8_lossy(&changed.stderr).contains("uninstall_installation_changed"));
        assert!(root.join(super::super::CLI_NAME).exists());
    }
}
