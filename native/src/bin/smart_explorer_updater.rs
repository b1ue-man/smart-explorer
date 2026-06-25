#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct ApplyArgs {
    target: PathBuf,
    staged: PathBuf,
    parent_pid: u32,
    version: String,
    last_applied: PathBuf,
    error_file: PathBuf,
    elevated: bool,
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    if !raw.iter().any(|a| a == "--apply") {
        return;
    }

    let fallback_error_file = arg_value(&raw, "--error-file")
        .map(PathBuf::from)
        .unwrap_or_else(default_error_file);
    match ApplyArgs::parse(&raw).and_then(apply_update) {
        Ok(()) => {}
        Err(e) => {
            record_failure(&fallback_error_file, &e);
            std::process::exit(1);
        }
    }
}

impl ApplyArgs {
    fn parse(raw: &[String]) -> Result<Self, String> {
        Ok(Self {
            target: PathBuf::from(required_arg(raw, "--target")?),
            staged: PathBuf::from(required_arg(raw, "--staged")?),
            parent_pid: required_arg(raw, "--parent-pid")?
                .parse()
                .map_err(|e| format!("parent pid ungueltig: {}", e))?,
            version: required_arg(raw, "--version")?,
            last_applied: PathBuf::from(required_arg(raw, "--last-applied")?),
            error_file: PathBuf::from(required_arg(raw, "--error-file")?),
            elevated: raw.iter().any(|a| a == "--elevated"),
        })
    }
}

fn apply_update(args: ApplyArgs) -> Result<(), String> {
    append_log(&format!(
        "apply v{}: staged={} target={} parent={}",
        args.version,
        args.staged.display(),
        args.target.display(),
        args.parent_pid
    ));

    let staged_len = std::fs::metadata(&args.staged)
        .map_err(|e| {
            format!(
                "gestagte Update-Datei fehlt ({}): {}",
                args.staged.display(),
                e
            )
        })?
        .len();

    if !wait_for_pid_exit(args.parent_pid, Duration::from_secs(30)) {
        append_log("parent did not exit within timeout; continuing with process cleanup");
    }
    if let Err(e) = stop_target_processes_for_update(&args.target) {
        if !args.elevated && e.needs_elevation {
            append_log("process cleanup needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|e| format!("Administratorfreigabe starten: {}", e))?;
            return Ok(());
        }
        return Err(e.msg);
    }

    let mut last_err = None;
    let mut last_needs_elevation = false;
    let mut replaced = false;
    for _ in 0..180 {
        match std::fs::copy(&args.staged, &args.target) {
            Ok(n) if n == staged_len => {
                replaced = true;
                break;
            }
            Ok(n) => {
                last_err = Some(format!(
                    "unvollstaendig kopiert: {} von {} Bytes",
                    n, staged_len
                ));
            }
            Err(e) => {
                last_needs_elevation = should_elevate_for_io(&e);
                last_err = Some(e.to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !replaced {
        let msg = format!(
            "Smart Explorer konnte nicht ersetzt werden: {}",
            last_err.unwrap_or_else(|| "unbekannter Fehler".to_string())
        );
        if !args.elevated && last_needs_elevation {
            append_log("copy needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|e| format!("Administratorfreigabe starten: {}", e))?;
            return Ok(());
        }
        return Err(msg);
    }

    if let Some(parent) = args.last_applied.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&args.last_applied, &args.version)
        .map_err(|e| format!("Update-Status schreiben: {}", e))?;

    let _ = std::fs::remove_file(&args.error_file);
    let _ = std::fs::remove_file(&args.staged);
    spawn_detached(&args.target, &["--updated"])
        .map_err(|e| format!("Smart Explorer neu starten: {}", e))?;
    append_log(&format!("apply v{}: ok", args.version));
    Ok(())
}

fn required_arg(raw: &[String], key: &str) -> Result<String, String> {
    arg_value(raw, key).ok_or_else(|| format!("Argument {} fehlt", key))
}

fn arg_value(raw: &[String], key: &str) -> Option<String> {
    raw.iter()
        .position(|a| a == key)
        .and_then(|i| raw.get(i + 1))
        .cloned()
}

fn appdata_dir() -> PathBuf {
    #[cfg(windows)]
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "linux")]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        });
    #[cfg(not(any(windows, target_os = "linux")))]
    let dir = std::env::temp_dir();
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app
}

fn default_error_file() -> PathBuf {
    appdata_dir().join("last_updater_error.txt")
}

fn record_failure(error_file: &Path, msg: &str) {
    if let Some(parent) = error_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(error_file, msg);
    append_log(&format!("error: {}", msg));
}

fn append_log(msg: &str) {
    let path = appdata_dir().join("updater.log");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 512 * 1024 {
        let _ = std::fs::write(&path, "");
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

#[derive(Debug)]
struct StopTargetError {
    msg: String,
    needs_elevation: bool,
}

impl StopTargetError {
    fn new(msg: impl Into<String>, needs_elevation: bool) -> Self {
        Self {
            msg: msg.into(),
            needs_elevation,
        }
    }
}

fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
    if pid == 0 {
        return true;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !h.is_null() {
                let rc = WaitForSingleObject(h, timeout.as_millis().min(u32::MAX as u128) as u32);
                CloseHandle(h);
                return rc == WAIT_OBJECT_0;
            }
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    true
}

fn request_daemon_stop_marker() {
    let sync = appdata_dir().join("sync");
    let _ = std::fs::create_dir_all(&sync);
    let _ = std::fs::write(sync.join("daemon.stop"), "stop");
}

fn clear_daemon_runtime_markers() {
    let sync = appdata_dir().join("sync");
    let _ = std::fs::remove_file(sync.join("daemon.heartbeat"));
    let _ = std::fs::remove_file(sync.join("daemon.ipc"));
}

#[cfg(not(windows))]
fn stop_target_processes_for_update(_target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    clear_daemon_runtime_markers();
    Ok(())
}

#[cfg(windows)]
fn stop_target_processes_for_update(target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    std::thread::sleep(Duration::from_millis(500));

    let natural_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let running = find_target_processes(target)?;
        if running.is_empty() || Instant::now() >= natural_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let running = find_target_processes(target)?;
    if !running.is_empty() {
        append_log(&format!(
            "terminating {} target process(es) before update",
            running.len()
        ));
    }
    for proc in running {
        terminate_process_for_update(proc.pid, &proc.image)?;
    }

    let forced_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = find_target_processes(target)?;
        if remaining.is_empty() {
            clear_daemon_runtime_markers();
            return Ok(());
        }
        if Instant::now() >= forced_deadline {
            let list = remaining
                .iter()
                .map(|p| format!("{} ({})", p.pid, p.image.display()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(StopTargetError::new(
                format!("Smart Explorer laeuft noch und blockiert das Update: {list}"),
                false,
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct TargetProcess {
    pid: u32,
    image: PathBuf,
}

#[cfg(windows)]
fn find_target_processes(target: &Path) -> Result<Vec<TargetProcess>, StopTargetError> {
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
    let mut matches = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(StopTargetError::new(
                format!("Prozessliste lesen: {}", std::io::Error::last_os_error()),
                is_last_error_elevation_related(),
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
                    match process_image_path(pid) {
                        Ok(Some(image)) if normalize_path_for_compare(&image) == target_norm => {
                            matches.push(TargetProcess { pid, image });
                        }
                        Ok(_) => {}
                        Err(e) if e.needs_elevation => {
                            CloseHandle(snapshot);
                            return Err(e);
                        }
                        Err(_) => {}
                    }
                }
            }
            ok = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    Ok(matches)
}

#[cfg(windows)]
fn process_image_path(pid: u32) -> Result<Option<PathBuf>, StopTargetError> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            if is_last_error_elevation_related() {
                return Err(StopTargetError::new(
                    format!(
                        "Prozess {pid} konnte nicht geprueft werden: {}",
                        std::io::Error::last_os_error()
                    ),
                    true,
                ));
            }
            return Ok(None);
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if !ok {
            if is_last_error_elevation_related() {
                return Err(StopTargetError::new(
                    format!(
                        "Prozesspfad fuer {pid} konnte nicht gelesen werden: {}",
                        std::io::Error::last_os_error()
                    ),
                    true,
                ));
            }
            return Ok(None);
        }
        buf.truncate(len as usize);
        Ok(Some(PathBuf::from(OsString::from_wide(&buf))))
    }
}

#[cfg(windows)]
fn terminate_process_for_update(pid: u32, image: &Path) -> Result<(), StopTargetError> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };

    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return Err(StopTargetError::new(
                format!(
                    "Prozess {pid} ({}) konnte nicht beendet werden: {}",
                    image.display(),
                    std::io::Error::last_os_error()
                ),
                is_last_error_elevation_related(),
            ));
        }
        let ok = TerminateProcess(h, 0) != 0;
        if !ok {
            let err = std::io::Error::last_os_error();
            CloseHandle(h);
            return Err(StopTargetError::new(
                format!(
                    "Prozess {pid} ({}) konnte nicht beendet werden: {err}",
                    image.display()
                ),
                is_last_error_elevation_related(),
            ));
        }
        let rc = WaitForSingleObject(h, 5000);
        CloseHandle(h);
        if rc == WAIT_OBJECT_0 {
            append_log(&format!("terminated process {pid} ({})", image.display()));
            Ok(())
        } else {
            Err(StopTargetError::new(
                format!(
                    "Prozess {pid} ({}) reagiert nach Terminate nicht",
                    image.display()
                ),
                false,
            ))
        }
    }
}

#[cfg(windows)]
fn wide_process_name(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(windows)]
fn is_last_error_elevation_related() -> bool {
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(5) | Some(740) | Some(1314)
    )
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

fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        let mut retry = std::process::Command::new(exe);
        retry
            .args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
        return retry.spawn().map(|_| ());
    }
    #[cfg(not(windows))]
    {
        cmd.spawn().map(|_| ())
    }
}

fn should_elevate_for_io(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(740) | Some(1314))
        || e.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(windows)]
fn relaunch_elevated(args: &ApplyArgs) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let argv = vec![
        "--apply".to_string(),
        "--target".to_string(),
        args.target.to_string_lossy().into_owned(),
        "--staged".to_string(),
        args.staged.to_string_lossy().into_owned(),
        "--parent-pid".to_string(),
        args.parent_pid.to_string(),
        "--version".to_string(),
        args.version.clone(),
        "--last-applied".to_string(),
        args.last_applied.to_string_lossy().into_owned(),
        "--error-file".to_string(),
        args.error_file.to_string_lossy().into_owned(),
        "--elevated".to_string(),
    ];
    spawn_elevated_detached(&exe, &argv)
}

#[cfg(not(windows))]
fn relaunch_elevated(_args: &ApplyArgs) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "elevation is only supported on Windows",
    ))
}

#[cfg(windows)]
fn spawn_elevated_detached(exe: &Path, args: &[String]) -> std::io::Result<()> {
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

#[cfg(windows)]
fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
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
