use std::path::{Path, PathBuf};
use std::time::Duration;

use super::archive::{archive_binary, exe_stem, new_old_binary_path, resume_auto_update, set_pin};
use super::config::{last_applied_path, updater_error_path};
use super::feed::Feed;

const INSTALLED_UPDATER: &str = "smart_explorer_updater";

/// The "rename dance" that swaps `new_exe` into the running binary's path.
/// Returns the path the caller should relaunch with `--updated`.
fn swap_in(new_exe: &Path) -> Result<PathBuf, String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let stem = exe_stem(&cur_exe);
    let pending = cur_exe.with_file_name(format!("{}_update_pending", stem));
    let old = new_old_binary_path(&cur_exe);

    std::fs::copy(new_exe, &pending).map_err(|e| format!("Kopieren fehlgeschlagen: {}", e))?;
    std::fs::rename(&cur_exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&pending);
        format!(
            "Programmdatei kann nicht ersetzt werden ({}): {}",
            cur_exe.display(),
            e
        )
    })?;
    if let Err(e) = std::fs::rename(&pending, &cur_exe) {
        let _ = std::fs::rename(&old, &cur_exe);
        let _ = std::fs::remove_file(&pending);
        return Err(format!("Einsetzen fehlgeschlagen: {}", e));
    }
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

fn copy_with_retries(src: &Path, dest: &Path, label: &str) -> Result<(), String> {
    let mut last = None;
    for _ in 0..10 {
        match std::fs::copy(src, dest) {
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
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "unbekannter Fehler".to_string())
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
            let result = copy_with_retries(&staged, &dest, "Updater-Helfer");
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
    spawn_detached(
        helper,
        &[
            "--apply",
            "--target",
            &target,
            "--staged",
            &staged,
            "--parent-pid",
            &parent_pid,
            "--version",
            version,
            "--last-applied",
            &last_applied,
            "--error-file",
            &error_file,
        ],
    )
    .map_err(|e| format!("Updater-Helfer starten: {}", e))?;
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
        if std::fs::copy(&src, &target).is_ok() {
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
