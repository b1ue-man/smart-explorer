use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::archive::{archive_binary, archived_sha256, pinned_version, restore_pin, set_pin};
use super::core::{replace_file_with_staged, staged_sha256_from_path, verify_sha256};
use super::feed::PayloadSpec;

const INSTALLED_APP: &str = "smart_explorer";
const INSTALLED_UPDATER: &str = "smart_explorer_updater";
const INSTALLED_CLI: &str = "se";

pub(super) fn create_startup_ack(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

pub(super) fn sync_startup_ack_parent(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

pub(super) fn publish_startup_ack(pending: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::hard_link(pending, final_path)?;
    if let Err(error) = sync_startup_ack_parent(final_path) {
        let _ = std::fs::remove_file(final_path);
        let _ = std::fs::remove_file(pending);
        return Err(error);
    }
    let _ = std::fs::remove_file(pending);
    let _ = sync_startup_ack_parent(final_path);
    Ok(())
}

pub(super) fn binary_suffix() -> &'static str {
    ""
}

pub(super) fn is_archived_binary(path: &Path) -> bool {
    path.is_file()
        && !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".sha256") || name.contains(".update-"))
}

pub(super) fn archived_name_without_binary_suffix(path: &Path) -> Option<&str> {
    path.file_name().and_then(|s| s.to_str())
}

pub(super) fn app_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["smart_explorer", "Smart Explorer"],
        http_names: &["smart_explorer", "Smart%20Explorer"],
        hash_name: "smart_explorer.sha256",
    }
}

pub(super) fn updater_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["smart_explorer_updater", "Smart Explorer Updater"],
        http_names: &["smart_explorer_updater", "Smart%20Explorer%20Updater"],
        hash_name: "smart_explorer_updater.sha256",
    }
}

pub(super) fn cli_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["se"],
        http_names: &["se"],
        hash_name: "se.sha256",
    }
}

/// The "rename dance" that swaps `new_exe` into the running binary's path.
/// Returns the path the caller should relaunch with `--updated`.
fn swap_in(new_exe: &Path, expected_sha256: &str) -> Result<PathBuf, String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    replace_file_with_staged(new_exe, &cur_exe, "Programmdatei", Some(expected_sha256))?;
    Ok(cur_exe)
}

pub(super) fn installed_updater_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_UPDATER))
}

pub(super) fn installed_cli_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_CLI))
}

/// App names that mark a desktop installation beside `se`.
pub(super) fn installed_app_names() -> &'static [&'static str] {
    &[INSTALLED_APP]
}

pub(super) fn installed_updater_name() -> &'static str {
    INSTALLED_UPDATER
}

/// The running `se` as the installed file: links such as `~/.local/bin/se`
/// are resolved, because the link itself must stay in place.
pub(super) fn running_cli_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("own path unknown: {e}"))?;
    std::fs::canonicalize(&exe).map_err(|error| format!("resolve {}: {error}", exe.display()))
}

/// The helper restarts the app with `--updated` and waits for its first frame.
pub(super) fn desktop_session() -> Result<(), String> {
    let display = std::env::var_os("DISPLAY");
    let wayland = std::env::var_os("WAYLAND_DISPLAY");
    if graphical_session(display.as_deref(), wayland.as_deref()) {
        Ok(())
    } else {
        Err(NO_GRAPHICAL_SESSION.to_string())
    }
}

const NO_GRAPHICAL_SESSION: &str = concat!(
    "no graphical session (DISPLAY and WAYLAND_DISPLAY are unset): the desktop updater ",
    "restarts Smart Explorer after replacing it, so run se update inside the desktop ",
    "session or use the terminal-only installation (install-linux.sh --cli-only)"
);

fn graphical_session(display: Option<&OsStr>, wayland: Option<&OsStr>) -> bool {
    [display, wayland]
        .into_iter()
        .any(|value| value.is_some_and(|value| !value.is_empty()))
}

/// Whether an update process that left a lock or leftovers still runs.
pub(super) fn process_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// A download is created without execute bits, but the staged helper is
/// started directly and staged binaries become installed executables.
pub(super) fn mark_staged_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("Update-Datei {} ausfuehrbar machen: {error}", path.display()))
}

/// A running Linux executable is replaced by renaming a new file over its
/// path; the running process keeps its old image.
pub(super) fn cli_self_replacement() -> Result<(), String> {
    Ok(())
}

pub(super) fn spawn_update_helper(
    helper: &Path,
    helper_sha256: &str,
    args: &[String],
    from_terminal: bool,
) -> Result<(), String> {
    verify_sha256(helper, helper_sha256)?;
    let mut command = std::process::Command::new(helper);
    command.args(args);
    if from_terminal {
        detach_from_terminal(&mut command);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Updater-Helfer starten: {error}"))
}

/// The helper and the app it restarts outlive `se update`: they must not
/// hold its pipes (`se update --json | jq` would wait for the app) or its
/// terminal, and a closed terminal must not hang them up.
fn detach_from_terminal(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid(2) is async-signal-safe, as a pre_exec hook requires.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

/// Revert to an archived binary.
pub fn revert_to(archived: &Path, version: &str) -> Result<PathBuf, String> {
    if !archived.exists() {
        return Err("Archivierte Version nicht gefunden".into());
    }
    let archived_hash = rollback_sha256(archived)?;
    archive_binary(env!("CARGO_PKG_VERSION"))?;
    let previous_pin = pinned_version();
    set_pin(version).map_err(|error| format!("Rollback-Pin speichern: {error}"))?;
    let cur_exe = match swap_in(archived, &archived_hash) {
        Ok(path) => path,
        Err(primary) => {
            return Err(match restore_pin(previous_pin.as_deref()) {
                Ok(()) => primary,
                Err(restore) => format!(
                    "{primary}; vorheriger Update-Pin konnte nicht wiederhergestellt werden: {restore}"
                ),
            });
        }
    };
    Ok(cur_exe)
}

fn rollback_sha256(path: &Path) -> Result<String, String> {
    if let Some(hash) = staged_sha256_from_path(path) {
        verify_sha256(path, &hash)?;
        Ok(hash)
    } else {
        archived_sha256(path)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn cli_task_staged_payload_is_executable_and_install_keeps_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"new se").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).unwrap();
        super::mark_staged_executable(&staged).unwrap();
        assert_eq!(mode(&staged), 0o755);

        let target = dir.path().join("se");
        std::fs::write(&target, b"old se").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sha256 = super::super::core::sha256_file(&staged).unwrap();
        let lock = super::super::cli_swap::UpdateLock::acquire(&target).unwrap();
        let install = super::super::cli_swap::install_cli_in_place;
        let replaced = install(&staged, &sha256, &target, lock).unwrap();
        replaced.commit().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new se");
        assert_eq!(mode(&target), 0o700);
    }

    #[test]
    fn cli_task_orphans_of_ended_updates_are_removed_and_stale_locks_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("se");
        std::fs::write(&target, b"se").unwrap();
        // PID 4294967294 cannot run on Linux (pid_max is at most 2^22).
        let dead = "4294967294";
        let own = std::process::id().to_string();
        for name in [
            format!("se.update-old.{dead}.1"),
            format!("se.update-pending.{dead}.2"),
            format!("se.update-old.{own}.3"),
            "se.notes".to_string(),
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        super::super::cli_swap::remove_orphans(&target);
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let own_backup = format!("se.update-old.{own}.3");
        assert_eq!(names, ["se", "se.notes", own_backup.as_str()]);

        std::fs::write(dir.path().join("se.update-lock"), dead).unwrap();
        let lock = super::super::cli_swap::UpdateLock::acquire(&target).unwrap();
        let owner = std::fs::read_to_string(dir.path().join("se.update-lock")).unwrap();
        assert_eq!(owner, own);
        drop(lock);
        assert!(!dir.path().join("se.update-lock").exists());
    }

    #[test]
    fn cli_task_desktop_update_needs_a_graphical_session() {
        use std::ffi::OsStr;
        assert!(super::graphical_session(Some(OsStr::new(":0")), None));
        assert!(super::graphical_session(None, Some(OsStr::new("wayland-0"))));
        assert!(!super::graphical_session(None, None));
        assert!(!super::graphical_session(Some(OsStr::new("")), None));
    }
}
