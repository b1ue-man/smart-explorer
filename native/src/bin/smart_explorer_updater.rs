#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::Duration;

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

    wait_for_pid_exit(args.parent_pid);

    let staged_len = std::fs::metadata(&args.staged)
        .map_err(|e| {
            format!(
                "gestagte Update-Datei fehlt ({}): {}",
                args.staged.display(),
                e
            )
        })?
        .len();
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

fn wait_for_pid_exit(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        const INFINITE: u32 = 0xFFFF_FFFF;
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !h.is_null() {
                WaitForSingleObject(h, INFINITE);
                CloseHandle(h);
                return;
            }
        }
    }
    std::thread::sleep(Duration::from_millis(300));
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
