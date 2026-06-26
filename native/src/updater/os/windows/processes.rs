use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) fn stop_target_processes_for_update(target: &Path) -> Result<(), String> {
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

pub(super) fn wait_for_pid_exit(pid: u32, timeout: Duration) {
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

fn clear_daemon_runtime_markers() {
    let sync = crate::support_dirs::sync_data_dir();
    let _ = std::fs::remove_file(sync.join("daemon.heartbeat"));
    let _ = std::fs::remove_file(sync.join("daemon.ipc"));
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
