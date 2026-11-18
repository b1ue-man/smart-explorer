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

#[path = "stop_loop.rs"]
mod stop_loop;
use stop_loop::Found;

/// The legacy stop request every worker and `se` process of this account
/// obeys. Written again when a fresh start consumed it (`stop_loop`).
pub(crate) fn request_daemon_stop_marker() -> Result<(), String> {
    let sync = appdata_dir().join("sync");
    std::fs::create_dir_all(&sync).map_err(|error| format!("{}: {error}", sync.display()))?;
    let marker = sync.join("daemon.stop");
    std::fs::write(&marker, "stop").map_err(|error| format!("{}: {error}", marker.display()))
}

/// Whether the stop request is still in place: an ordinary worker start
/// discards it once it holds the singleton, a handoff replaces it.
fn stop_request_pending() -> bool {
    std::fs::read_to_string(appdata_dir().join("sync/daemon.stop"))
        .is_ok_and(|value| value.trim() == "stop")
}

fn clear_daemon_runtime_markers() {
    let sync = appdata_dir().join("sync");
    let _ = std::fs::remove_file(sync.join("daemon.heartbeat"));
    let _ = std::fs::remove_file(sync.join("daemon.ipc"));
}

#[cfg(target_os = "linux")]
pub(crate) fn target_is_running(target: &Path) -> Result<bool, StopTargetError> {
    linux_target_pids(target).map(|running| !running.is_empty())
}

#[cfg(windows)]
pub(crate) fn target_is_running(target: &Path) -> Result<bool, StopTargetError> {
    let found = find_target_processes(target)?;
    if found
        .iter()
        .any(|process| matches!(process, Found::Target { .. }))
    {
        return Ok(true);
    }
    if found.is_empty() {
        return Ok(false);
    }
    Err(stop_loop::report(
        &found,
        0,
        super::installer::is_elevated_context(),
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn target_is_running(_target: &Path) -> Result<bool, StopTargetError> {
    Err(StopTargetError::new(
        "Prozesspruefung wird auf diesem Betriebssystem nicht unterstuetzt",
        false,
    ))
}

/// `stop_target_processes_with` that renews the plain legacy request.
pub(crate) fn stop_target_processes_for_update(target: &Path) -> Result<(), StopTargetError> {
    stop_target_processes_with(target, &mut request_daemon_stop_marker)
}

/// Asks every process of `target` to exit (unless the identity-bound update
/// transaction already published the request) and waits until none runs.
/// `restop` renews the request when a fresh start consumed it.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn stop_target_processes_with(
    target: &Path,
    restop: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), StopTargetError> {
    let marker = appdata_dir().join("sync/daemon.stop");
    let owned = target.parent().is_some_and(|root| {
        smart_explorer::mcp_launcher::stop_marker_is_owned(root, &marker).unwrap_or(false)
    });
    if !owned {
        let _ = request_daemon_stop_marker();
    }
    let result = stop_loop::stop_until_gone(
        stop_loop::LIMITS,
        super::installer::is_elevated_context(),
        &mut || scan_target(target),
        &stop_request_pending,
        restop,
    );
    if result.is_ok() {
        clear_daemon_runtime_markers();
    }
    result
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn stop_target_processes_with(
    _target: &Path,
    _restop: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), StopTargetError> {
    Err(StopTargetError::new(
        "Prozesspruefung wird auf diesem Betriebssystem nicht unterstuetzt",
        false,
    ))
}

#[cfg(target_os = "linux")]
fn scan_target(target: &Path) -> Result<Vec<Found>, StopTargetError> {
    let image = target.display().to_string();
    Ok(linux_target_pids(target)?
        .into_iter()
        .map(|pid| Found::Target {
            pid,
            image: image.clone(),
        })
        .collect())
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

#[cfg(windows)]
fn scan_target(target: &Path) -> Result<Vec<Found>, StopTargetError> {
    find_target_processes(target)
}

/// Processes of `target`'s image, and processes of its file name whose image
/// this helper may not read (another account, elevated, protected, or in the
/// middle of its exit): those are kept with PID and error code instead of
/// ending the cleanup at once as "needs administrator rights".
#[cfg(windows)]
fn find_target_processes(target: &Path) -> Result<Vec<Found>, StopTargetError> {
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
            let error = std::io::Error::last_os_error();
            return Err(StopTargetError::new(
                format!("Prozessliste lesen: {error}"),
                error.raw_os_error().is_some_and(stop_loop::elevation_code),
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
                        ImageProbe::Path(image)
                            if normalize_path_for_compare(&image) == target_norm =>
                        {
                            matches.push(Found::Target {
                                pid,
                                image: image.display().to_string(),
                            });
                        }
                        ImageProbe::Unreadable(code) => {
                            matches.push(Found::Unreadable { pid, code })
                        }
                        ImageProbe::Path(_) | ImageProbe::Gone => {}
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
enum ImageProbe {
    Path(std::path::PathBuf),
    /// Exited meanwhile, or failed with an error that is no access refusal
    /// (such processes were skipped before as well).
    Gone,
    /// The OS refused access (5, 740 or 1314): not readable for this account.
    Unreadable(i32),
}

#[cfg(windows)]
fn process_image_path(pid: u32) -> ImageProbe {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // The code is read right after the failing call, before CloseHandle.
    let refused = |code: i32| {
        if stop_loop::elevation_code(code) {
            ImageProbe::Unreadable(code)
        } else {
            ImageProbe::Gone
        }
    };
    let last_code = || std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return refused(last_code());
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        let code = if ok { 0 } else { last_code() };
        CloseHandle(h);
        if !ok {
            return refused(code);
        }
        buf.truncate(len as usize);
        ImageProbe::Path(std::path::PathBuf::from(OsString::from_wide(&buf)))
    }
}

#[cfg(windows)]
fn wide_process_name(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
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
