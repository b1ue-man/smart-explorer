use std::path::{Path, PathBuf};

use super::archive::{archive_binary, archived_sha256, pinned_version, restore_pin, set_pin};
use super::core::{replace_file_with_staged, staged_sha256_from_path, verify_sha256};
use super::feed::PayloadSpec;

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

pub(super) fn spawn_update_helper(
    helper: &Path,
    helper_sha256: &str,
    args: &[String],
) -> Result<(), String> {
    verify_sha256(helper, helper_sha256)?;
    std::process::Command::new(helper)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Updater-Helfer starten: {error}"))
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
