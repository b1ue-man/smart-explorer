use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::archive::{archive_binary, exe_stem, new_old_binary_path, resume_auto_update, set_pin};
use super::config::{last_applied_path, updater_error_path};
use super::feed::Feed;

const INSTALLED_UPDATER_EXE: &str = "Smart Explorer Updater.exe";
const SHARE_FIREWALL_RULE: &str = "Smart Explorer Share Peer Listener";

/// The "rename dance" that swaps `new_exe` into the running binary's path.
/// Returns the path the caller should relaunch with `--updated`.
fn swap_in(new_exe: &Path) -> Result<PathBuf, String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let stem = exe_stem(&cur_exe);
    let pending = cur_exe.with_file_name(format!("{}_update_pending.exe", stem));
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
}

fn copy_with_retries(src: &Path, dest: &Path, label: &str) -> Result<(), CopyRetryError> {
    let mut last = None;
    let mut needs_elevation = false;
    for _ in 0..10 {
        match std::fs::copy(src, dest) {
            Ok(_) => return Ok(()),
            Err(e) => {
                needs_elevation |= should_elevate_for_spawn(&e);
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
            last.map(|e| e.to_string())
                .unwrap_or_else(|| "unbekannter Fehler".to_string())
        ),
        needs_elevation,
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
        Ok(staged) => match copy_with_retries(&staged, &dest, "Updater-Helfer") {
            Ok(()) => {
                let _ = std::fs::remove_file(&staged);
                Ok(dest)
            }
            Err(e) if e.needs_elevation || dest.exists() => Ok(staged),
            Err(e) => {
                let _ = std::fs::remove_file(&staged);
                Err(e.msg)
            }
        },
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
    request_daemon_stop_for_update();
    let _ = stop_target_processes_for_update(&target);

    let mut replaced = false;
    for _ in 0..60 {
        if std::fs::copy(&src, &target).is_ok() {
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

fn clear_daemon_runtime_markers() {
    let sync = crate::support_dirs::sync_data_dir();
    let _ = std::fs::remove_file(sync.join("daemon.heartbeat"));
    let _ = std::fs::remove_file(sync.join("daemon.ipc"));
}

fn stop_target_processes_for_update(target: &Path) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(500));
    let natural_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let running = find_target_processes(target)?;
        if running.is_empty() || Instant::now() >= natural_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    for proc in find_target_processes(target)? {
        terminate_target_process(proc.pid, &proc.image)?;
    }
    clear_daemon_runtime_markers();
    Ok(())
}

#[derive(Debug)]
struct TargetProcess {
    pid: u32,
    image: PathBuf,
}

fn find_target_processes(target: &Path) -> Result<Vec<TargetProcess>, String> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let target_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let target_norm = normalize_path_for_compare(target);
    let mut out = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "Prozessliste lesen: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
        while ok {
            let pid = entry.th32ProcessID;
            if pid != std::process::id() {
                let exe_name = wide_process_name(&entry.szExeFile).to_ascii_lowercase();
                if exe_name == target_name {
                    if let Some(image) = process_image_path(pid)? {
                        if normalize_path_for_compare(&image) == target_norm {
                            out.push(TargetProcess { pid, image });
                        }
                    }
                }
            }
            ok = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    Ok(out)
}

fn process_image_path(pid: u32) -> Result<Option<PathBuf>, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return Ok(None);
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if !ok {
            return Ok(None);
        }
        buf.truncate(len as usize);
        Ok(Some(PathBuf::from(OsString::from_wide(&buf))))
    }
}

fn terminate_target_process(pid: u32, image: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return Err(format!(
                "Smart Explorer Prozess {pid} ({}) konnte nicht beendet werden: {}",
                image.display(),
                std::io::Error::last_os_error()
            ));
        }
        if TerminateProcess(h, 0) == 0 {
            let err = std::io::Error::last_os_error();
            CloseHandle(h);
            return Err(format!(
                "Smart Explorer Prozess {pid} ({}) konnte nicht beendet werden: {err}",
                image.display()
            ));
        }
        let rc = WaitForSingleObject(h, 5000);
        CloseHandle(h);
        if rc == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(format!(
                "Smart Explorer Prozess {pid} ({}) reagiert nach Terminate nicht",
                image.display()
            ))
        }
    }
}

fn wide_process_name(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn normalize_path_for_compare(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_path_string(&path.to_string_lossy())
}

fn normalize_path_string(path: &str) -> String {
    let mut s = path.replace('/', "\\");
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        s = stripped.to_string();
    }
    s.trim_end_matches('\\').to_lowercase()
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    if pid == 0 {
        return;
    }
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, timeout.as_millis() as u32);
            CloseHandle(h);
            return;
        }
    }
    std::thread::sleep(Duration::from_millis(300));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_string_matches_windows_variants() {
        assert_eq!(
            normalize_path_string(r"\\?\C:\Program Files\Smart Explorer\smart_explorer.exe\"),
            r"c:\program files\smart explorer\smart_explorer.exe"
        );
        assert_eq!(
            normalize_path_string("C:/Program Files/Smart Explorer/smart_explorer.exe"),
            r"c:\program files\smart explorer\smart_explorer.exe"
        );
    }
}
