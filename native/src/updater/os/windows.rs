use std::path::{Path, PathBuf};
use std::time::Duration;

use super::archive::{archive_binary, resume_auto_update, set_pin};
use super::config::{last_applied_path, updater_error_path};
use super::core::{replace_file_with_staged, staged_sha256_from_path, verify_sha256};
use super::feed::{Feed, PayloadSpec};

#[path = "windows/processes.rs"]
mod processes;

use processes::{stop_target_processes_for_update, wait_for_pid_exit};

const INSTALLED_UPDATER_EXE: &str = "Smart Explorer Updater.exe";
const SHARE_FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

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

/// Spawn a process fully detached: no console window, and broken away from any
/// job object so it outlives this process.
fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
    match cmd.spawn() {
        Ok(_) => return Ok(()),
        Err(e) if should_elevate_for_spawn(&e) => {
            return spawn_elevated_detached(exe, args);
        }
        Err(_) => {}
    }
    let mut c2 = std::process::Command::new(exe);
    c2.args(args)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    match c2.spawn() {
        Ok(_) => Ok(()),
        Err(e) if should_elevate_for_spawn(&e) => spawn_elevated_detached(exe, args),
        Err(e) => Err(e),
    }
}

fn should_elevate_for_spawn(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(740) | Some(1314))
        || e.kind() == std::io::ErrorKind::PermissionDenied
}

fn spawn_elevated_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide_os(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let verb = wide_str("runas");
    let file = wide_os(exe.as_os_str());
    let params = wide_str(&join_windows_args(args));
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if rc > 32 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Administratorfreigabe abgebrochen oder verweigert (ShellExecuteW={rc})"),
        ))
    }
}

fn join_windows_args(args: &[&str]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '"'))
    {
        return arg.to_string();
    }

    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn installed_updater_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_UPDATER_EXE))
}

struct CopyRetryError {
    msg: String,
    needs_elevation: bool,
    integrity_error: bool,
}

fn copy_with_retries(
    src: &Path,
    dest: &Path,
    label: &str,
    expected_sha256: Option<&str>,
) -> Result<(), CopyRetryError> {
    let mut last = None;
    let mut needs_elevation = false;
    let mut integrity_error = false;
    for _ in 0..10 {
        match replace_file_with_staged(src, dest, label, expected_sha256) {
            Ok(_) => return Ok(()),
            Err(e) => {
                needs_elevation |= looks_like_elevation_error(&e);
                integrity_error |= looks_like_integrity_error(&e);
                last = Some(e);
                std::thread::sleep(Duration::from_millis(350));
            }
        }
    }
    Err(CopyRetryError {
        msg: format!(
            "{} kopieren ({} -> {}): {}",
            label,
            src.display(),
            dest.display(),
            last.unwrap_or_else(|| "unbekannter Fehler".to_string())
        ),
        needs_elevation,
        integrity_error,
    })
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
            match copy_with_retries(&staged, &dest, "Updater-Helfer", expected_sha256.as_deref()) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&staged);
                    Ok(dest)
                }
                Err(e) if !e.integrity_error && (e.needs_elevation || dest.exists()) => Ok(staged),
                Err(e) => {
                    let _ = std::fs::remove_file(&staged);
                    Err(e.msg)
                }
            }
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
    request_daemon_stop_for_update();
    let _ = stop_target_processes_for_update(&target);

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
        let _ = ensure_share_firewall_rule_for(&target);
        let _ = spawn_detached(&target, &["--updated"]);
    }
}

fn looks_like_elevation_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("zugriff")
        || lower.contains("permission")
        || lower.contains("os error 5")
}

fn looks_like_integrity_error(msg: &str) -> bool {
    msg.contains("Pruefsumme") || msg.contains("SHA-256") || msg.contains("Hash-Datei")
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

fn request_daemon_stop_for_update() {
    let sync = crate::support_dirs::sync_data_dir();
    let _ = std::fs::write(sync.join("daemon.stop"), "stop");
}

/// Revert to an archived binary.
pub fn revert_to(archived: &Path, version: &str) -> Result<PathBuf, String> {
    if !archived.exists() {
        return Err("Archivierte Version nicht gefunden".into());
    }
    archive_binary(env!("CARGO_PKG_VERSION"));
    let cur_exe = swap_in(archived)?;
    let _ = ensure_share_firewall_rule_for(&cur_exe);
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
    let _ = ensure_share_firewall_rule_for(&cur_exe);
    resume_auto_update();
    Ok(cur_exe)
}
