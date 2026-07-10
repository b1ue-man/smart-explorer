use std::path::{Path, PathBuf};

use super::archive::{archive_binary, archived_sha256, pinned_version, restore_pin, set_pin};
use super::core::{replace_file_with_staged, staged_sha256_from_path, verify_sha256};
use super::feed::PayloadSpec;

const INSTALLED_UPDATER_EXE: &str = "Smart Explorer Updater.exe";
const INSTALLED_CLI_EXE: &str = "se.exe";
const SHARE_FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

pub(super) fn create_startup_ack(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

pub(super) fn publish_startup_ack(pending: &Path, final_path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let pending = pending
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let final_path = final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            pending.as_ptr(),
            final_path.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn binary_suffix() -> &'static str {
    ".exe"
}

pub(super) fn is_archived_binary(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()) == Some("exe")
}

pub(super) fn archived_name_without_binary_suffix(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|s| s.to_str())
}

pub(super) fn app_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["smart_explorer.exe", "Smart Explorer.exe"],
        http_names: &["smart_explorer.exe", "Smart%20Explorer.exe"],
        hash_name: "smart_explorer.exe.sha256",
    }
}

pub(super) fn updater_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["smart_explorer_updater.exe", "Smart Explorer Updater.exe"],
        http_names: &[
            "smart_explorer_updater.exe",
            "Smart%20Explorer%20Updater.exe",
        ],
        hash_name: "smart_explorer_updater.exe.sha256",
    }
}

pub(super) fn cli_payload_spec() -> PayloadSpec {
    PayloadSpec {
        local_names: &["se.exe"],
        http_names: &["se.exe"],
        hash_name: "se.exe.sha256",
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
    Ok(dir.join(INSTALLED_UPDATER_EXE))
}

pub(super) fn installed_cli_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_CLI_EXE))
}

pub(super) fn spawn_update_helper(
    helper: &Path,
    helper_sha256: &str,
    args: &[String],
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    verify_sha256(helper, helper_sha256)?;
    let mut command = std::process::Command::new(helper);
    command
        .args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
    match command.spawn() {
        Ok(_) => Ok(()),
        Err(_) => {
            verify_sha256(helper, helper_sha256)?;
            let mut retry = std::process::Command::new(helper);
            retry
                .args(args)
                .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
            match retry.spawn() {
                Ok(_) => Ok(()),
                Err(error)
                    if matches!(error.raw_os_error(), Some(5) | Some(740) | Some(1314))
                        || error.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    Err(format!(
                        "Updater-Helfer benoetigt Administratorrechte; bitte den Installer verwenden: {error}"
                    ))
                }
                Err(error) => Err(format!("Updater-Helfer starten: {error}")),
            }
        }
    }
}

fn ensure_share_firewall_rule_for(exe: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = exe.to_string_lossy().to_string();
    let _ = std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={SHARE_FIREWALL_RULE}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let output = std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={SHARE_FIREWALL_RULE}"),
            "dir=in",
            "action=allow",
            &format!("program={exe}"),
            "enable=yes",
            "profile=any",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
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
    let _ = ensure_share_firewall_rule_for(&cur_exe);
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
