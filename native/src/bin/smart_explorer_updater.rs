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
            Err(e) => last_err = Some(e.to_string()),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !replaced {
        return Err(format!(
            "Smart Explorer.exe konnte nicht ersetzt werden: {}",
            last_err.unwrap_or_else(|| "unbekannter Fehler".to_string())
        ));
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
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
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
