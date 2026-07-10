use super::logging::appdata_dir;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct StopTargetError {
    pub(crate) msg: String,
    pub(crate) needs_elevation: bool,
}

impl StopTargetError {
    fn new(msg: impl Into<String>, needs_elevation: bool) -> Self {
        Self {
            msg: msg.into(),
            needs_elevation,
        }
    }
}

pub(crate) fn wait_for_pid_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(87) {
                    return Ok(());
                }
                return Err(format!(
                    "Elternprozess {pid} konnte nicht ueberwacht werden: {error}"
                ));
            }
            let rc = WaitForSingleObject(h, timeout.as_millis().min(u32::MAX as u128) as u32);
            CloseHandle(h);
            match rc {
                WAIT_OBJECT_0 => Ok(()),
                WAIT_TIMEOUT => Err(format!(
                    "Elternprozess {pid} lief nach {} Sekunden noch; Update wurde nicht angewendet",
                    timeout.as_secs()
                )),
                _ => Err(format!("Warten auf Elternprozess {pid} fehlgeschlagen")),
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let rc = unsafe { libc::kill(pid as i32, 0) };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(());
                }
                if error.raw_os_error() != Some(libc::EPERM) {
                    return Err(format!("Elternprozess {pid} pruefen: {error}"));
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "Elternprozess {pid} lief nach {} Sekunden noch; Update wurde nicht angewendet",
                    timeout.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    return Err("PID-Warten wird auf diesem Betriebssystem nicht unterstuetzt".to_string());
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

#[cfg(target_os = "linux")]
pub(crate) fn stop_target_processes_for_update(target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let running = linux_target_pids(target)?;
        if running.is_empty() {
            clear_daemon_runtime_markers();
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(StopTargetError::new(
                format!(
                    "Smart Explorer laeuft noch und blockiert das Update (PIDs: {})",
                    running
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                false,
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(target_os = "linux")]
fn linux_target_pids(target: &Path) -> Result<Vec<u32>, StopTargetError> {
    let expected = std::fs::canonicalize(target).map_err(|error| {
        StopTargetError::new(
            format!("Programmdatei {} aufloesen: {error}", target.display()),
            false,
        )
    })?;
    let mut matches = Vec::new();
    let entries = std::fs::read_dir("/proc").map_err(|error| {
        StopTargetError::new(format!("Linux-Prozessliste lesen: {error}"), false)
    })?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        if std::fs::read_link(entry.path().join("exe"))
            .ok()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .is_some_and(|path| path == expected)
        {
            matches.push(pid);
        }
    }
    Ok(matches)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn stop_target_processes_for_update(_target: &Path) -> Result<(), StopTargetError> {
    Err(StopTargetError::new(
        "Prozesspruefung wird auf diesem Betriebssystem nicht unterstuetzt",
        false,
    ))
}

#[cfg(windows)]
pub(crate) fn stop_target_processes_for_update(target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    std::thread::sleep(Duration::from_millis(500));

    let natural_deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let running = find_target_processes(target)?;
        if running.is_empty() || std::time::Instant::now() >= natural_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let remaining = find_target_processes(target)?;
    if remaining.is_empty() {
        clear_daemon_runtime_markers();
        return Ok(());
    }
    let list = remaining
        .iter()
        .map(|process| format!("{} ({})", process.pid, process.image.display()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(StopTargetError::new(
        format!("Smart Explorer laeuft noch und blockiert das Update: {list}"),
        false,
    ))
}

#[cfg(windows)]
#[derive(Debug)]
struct TargetProcess {
    pid: u32,
    image: std::path::PathBuf,
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
fn process_image_path(pid: u32) -> Result<Option<std::path::PathBuf>, StopTargetError> {
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
        Ok(Some(std::path::PathBuf::from(OsString::from_wide(&buf))))
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

#[cfg(windows)]
fn normalize_path_for_compare(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_path_string(&path.to_string_lossy())
}

#[cfg(windows)]
fn normalize_path_string(path: &str) -> String {
    let mut s = path.replace('/', "\\");
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        s = stripped.to_string();
    }
    s.trim_end_matches('\\').to_lowercase()
}

#[cfg(all(test, windows))]
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
