use std::path::{Path, PathBuf};
use std::time::Duration;

use super::archive::{archive_binary, resume_auto_update, set_pin};
use super::config::{last_applied_path, updater_error_path};
use super::core::{replace_file_with_staged, staged_sha256_from_path, verify_sha256};
use super::feed::{Feed, PayloadSpec};

const INSTALLED_UPDATER: &str = "smart_explorer_updater";

pub(super) fn binary_suffix() -> &'static str {
    ""
}

pub(super) fn is_archived_binary(path: &Path) -> bool {
    path.is_file()
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

/// The "rename dance" that swaps `new_exe` into the running binary's path.
/// Returns the path the caller should relaunch with `--updated`.
fn swap_in(new_exe: &Path) -> Result<PathBuf, String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let expected_sha256 = staged_sha256_from_path(new_exe);
    replace_file_with_staged(
        new_exe,
        &cur_exe,
        "Programmdatei",
        expected_sha256.as_deref(),
    )?;
    Ok(cur_exe)
}

fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    cmd.spawn().map(|_| ())
}

fn installed_updater_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_UPDATER))
}

fn copy_with_retries(
    src: &Path,
    dest: &Path,
    label: &str,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    let mut last = None;
    for _ in 0..10 {
        match replace_file_with_staged(src, dest, label, expected_sha256) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(350));
            }
        }
    }
    Err(format!(
        "{} kopieren ({} -> {}): {}",
        label,
        src.display(),
        dest.display(),
        last.unwrap_or_else(|| "unbekannter Fehler".to_string())
    ))
}

pub(super) fn ensure_installed_updater(
    feed: &Feed,
    version: &str,
    refresh: bool,
) -> Result<PathBuf, String> {
    let dest = installed_updater_path()?;
    if !refresh && dest.exists() {
        return Ok(dest);
    }

    match feed.fetch_updater_exe(version) {
        Ok(staged) => {
            let expected_sha256 = staged_sha256_from_path(&staged);
            let result =
                copy_with_retries(&staged, &dest, "Updater-Helfer", expected_sha256.as_deref());
            let _ = std::fs::remove_file(&staged);
            result?;
            Ok(dest)
        }
        Err(_e) if dest.exists() => Ok(dest),
        Err(e) => Err(format!(
            "Updater-Helfer fehlt und konnte nicht aus dem Feed geladen werden: {}",
            e
        )),
    }
}

pub(super) fn apply_via_installed_updater(
    helper: &Path,
    staged_exe: &Path,
    version: &str,
) -> Result<(), String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let target = cur_exe.to_string_lossy().into_owned();
    let staged = staged_exe.to_string_lossy().into_owned();
    let parent_pid = std::process::id().to_string();
    let last_applied = last_applied_path().to_string_lossy().into_owned();
    let error_file = updater_error_path().to_string_lossy().into_owned();
    if let Some(hash) = staged_sha256_from_path(staged_exe) {
        verify_sha256(staged_exe, &hash)?;
    }
    if let Some(hash) = staged_sha256_from_path(helper) {
        verify_sha256(helper, &hash)?;
    }
    let mut argv = vec![
        "--apply".to_string(),
        "--target".to_string(),
        target,
        "--staged".to_string(),
        staged,
        "--parent-pid".to_string(),
        parent_pid,
        "--version".to_string(),
        version.to_string(),
        "--last-applied".to_string(),
        last_applied,
        "--error-file".to_string(),
        error_file,
    ];
    if let Some(hash) = staged_sha256_from_path(staged_exe) {
        argv.push("--staged-sha256".to_string());
        argv.push(hash);
    }
    if let Some(hash) = staged_sha256_from_path(helper) {
        argv.push("--helper-sha256".to_string());
        argv.push(hash);
    }
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    spawn_detached(helper, &argv_refs).map_err(|e| format!("Updater-Helfer starten: {}", e))?;
    Ok(())
}

/// Worker entry point (`--apply-update <target> <parent_pid>`).
pub fn run_apply_worker(args: &[String]) {
    let i = match args.iter().position(|a| a == "--apply-update") {
        Some(i) => i,
        None => return,
    };
    let target = match args.get(i + 1) {
        Some(t) => PathBuf::from(t),
        None => return,
    };
    let parent_pid: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let src = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    wait_for_pid_exit(parent_pid, Duration::from_secs(30));

    let mut replaced = false;
    for _ in 0..60 {
        let expected_sha256 = staged_sha256_from_path(&src);
        if replace_file_with_staged(&src, &target, "Apply-Worker", expected_sha256.as_deref())
            .is_ok()
        {
            replaced = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if replaced {
        let _ = spawn_detached(&target, &["--updated"]);
    }
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    if pid == 0 {
        return;
    }
    let _ = timeout;
    std::thread::sleep(Duration::from_millis(300));
}

/// Revert to an archived binary.
pub fn revert_to(archived: &Path, version: &str) -> Result<PathBuf, String> {
    if !archived.exists() {
        return Err("Archivierte Version nicht gefunden".into());
    }
    archive_binary(env!("CARGO_PKG_VERSION"));
    let cur_exe = swap_in(archived)?;
    set_pin(version);
    Ok(cur_exe)
}

/// Install a downloaded released binary as a forward update.
pub fn install_version(downloaded: &Path, version: &str) -> Result<PathBuf, String> {
    if !downloaded.exists() {
        return Err("Heruntergeladene Version nicht gefunden".into());
    }
    archive_binary(env!("CARGO_PKG_VERSION"));
    if let Ok(helper) = installed_updater_path() {
        if helper.exists() {
            resume_auto_update();
            apply_via_installed_updater(&helper, downloaded, version)?;
            return Ok(PathBuf::new());
        }
    }
    let cur_exe = swap_in(downloaded)?;
    resume_auto_update();
    Ok(cur_exe)
}
