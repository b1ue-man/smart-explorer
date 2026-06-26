use super::logging::{appdata_dir, append_log};
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

pub(crate) fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
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
pub(crate) fn stop_target_processes_for_update(_target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    clear_daemon_runtime_markers();
    Ok(())
}

#[cfg(windows)]
pub(crate) fn stop_target_processes_for_update(target: &Path) -> Result<(), StopTargetError> {
    request_daemon_stop_marker();
    std::thread::sleep(Duration::from_millis(500));

    let natural_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let running = find_target_processes(target)?;
        if running.is_empty() || std::time::Instant::now() >= natural_deadline {
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

    let forced_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = find_target_processes(target)?;
        if remaining.is_empty() {
            clear_daemon_runtime_markers();
            return Ok(());
        }
        if std::time::Instant::now() >= forced_deadline {
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
