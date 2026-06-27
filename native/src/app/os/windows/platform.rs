use super::super::prelude::*;
use super::super::support_paths::OpenMode;
use super::super::*;
use super::shared_platform_helpers::{ClipboardEffect, ClipboardVirtualFile};

pub(in crate::app) fn confirm_yes_no(title: &str, msg: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO};
    let t: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let m: Vec<u16> = msg.encode_utf16().chain(Some(0)).collect();
    let r = unsafe {
        MessageBoxW(
            None,
            PCWSTR(m.as_ptr()),
            PCWSTR(t.as_ptr()),
            MB_YESNO | MB_ICONWARNING,
        )
    };
    r == IDYES
}

pub(in crate::app) fn app_is_foreground() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid != 0 && pid == GetCurrentProcessId()
    }
}

pub(in crate::app) fn list_drives() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let bits = unsafe { GetLogicalDrives() };
    (0u32..26)
        .filter(|i| bits & (1 << i) != 0)
        .map(|i| format!("{}:\\", char::from(b'A' + i as u8)))
        .collect()
}

pub(in crate::app) fn drive_info_list(drives: &[String]) -> Vec<(String, u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    drives
        .iter()
        .map(|d| {
            let wide: Vec<u16> = d.encode_utf16().chain(Some(0)).collect();
            let mut free = 0u64;
            let mut total = 0u64;
            let mut total_free = 0u64;
            unsafe {
                GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free);
            }
            (d.clone(), free, total)
        })
        .collect()
}

pub(in crate::app) fn reveal_path_in_file_manager(path: &str) {
    let p = path.replace('/', "\\");
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", p))
        .spawn();
}

fn shell_execute_path(path: &str, verb: Option<&str>) {
    let p = path.replace('/', "\\");
    let wide: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
    let verb_w: Option<Vec<u16>> = verb.map(|v| v.encode_utf16().chain(Some(0)).collect());
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
    }
}

pub(in crate::app) fn open_local_path(path: &str, mode: OpenMode) {
    match mode {
        OpenMode::Default => shell_execute_path(path, None),
        OpenMode::With => shell_execute_path(path, Some("openas")),
    }
}

pub(in crate::app) struct EditProcess {
    pub(in crate::app) handle: windows_sys::Win32::Foundation::HANDLE,
}

impl EditProcess {
    pub(in crate::app) fn new(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Self> {
        if handle.is_null() {
            None
        } else {
            Some(Self { handle })
        }
    }

    pub(in crate::app) fn is_finished(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for EditProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

pub(in crate::app) fn launch_local_for_edit(path: &str, mode: OpenMode) -> Option<EditProcess> {
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    let p = path.replace('/', "\\");
    let file: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
    let verb: Option<Vec<u16>> = match mode {
        OpenMode::Default => None,
        OpenMode::With => Some("openas".encode_utf16().chain(Some(0)).collect()),
    };
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    info.lpFile = file.as_ptr();
    info.nShow = 1;
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        None
    } else {
        EditProcess::new(info.hProcess)
    }
}

pub(in crate::app) fn open_terminal_at(path: &str) {
    if path.is_empty() {
        return;
    }
    let dir = path.replace('/', "\\");
    let dir_w: Vec<u16> = dir.encode_utf16().chain(Some(0)).collect();
    let file_w: Vec<u16> = "cmd.exe".encode_utf16().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            file_w.as_ptr(),
            std::ptr::null(),
            dir_w.as_ptr(),
            1,
        );
    }
}

pub(in crate::app) fn spawn_updated_app(exe: &Path) {
    let _ = std::process::Command::new(exe).arg("--updated").spawn();
}

pub(in crate::app) fn update_payload_name() -> &'static str {
    "smart_explorer.exe"
}

pub(in crate::app) fn shell_integration_available() -> bool {
    true
}

pub(in crate::app) fn set_context_menu_enabled(on: bool) -> std::io::Result<()> {
    crate::shell_register::set_context_menu(on)
}

pub(in crate::app) fn show_properties_for_path(path: &str) {
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    };
    let p = path.replace('/', "\\");
    let verb: Vec<u16> = "properties".encode_utf16().chain(Some(0)).collect();
    let file: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.nShow = 1;
    unsafe {
        ShellExecuteExW(&mut info);
    }
}

pub(in crate::app) fn write_clipboard_files(
    paths: &[String],
    effect: ClipboardEffect,
) -> Result<(), String> {
    let drop_effect = match effect {
        ClipboardEffect::Copy => crate::shell_clipboard::DROPEFFECT_COPY,
        ClipboardEffect::Move => crate::shell_clipboard::DROPEFFECT_MOVE,
    };
    crate::shell_clipboard::write_files(paths, drop_effect).map_err(|e| e.to_string())
}

pub(in crate::app) fn read_clipboard_files() -> Option<(Vec<String>, bool)> {
    crate::shell_clipboard::read_files()
}

pub(in crate::app) fn set_virtual_clipboard(
    files: Vec<ClipboardVirtualFile>,
) -> Result<u32, String> {
    let files = files
        .into_iter()
        .map(|f| crate::virtual_clipboard::VirtualFile {
            abs: f.abs,
            rel: f.rel,
            size: f.size,
            mtime_ms: f.mtime_ms,
        })
        .collect();
    crate::virtual_clipboard::set_clipboard(files).map_err(|e| e.to_string())
}

pub(in crate::app) fn virtual_clipboard_sequence() -> Option<u32> {
    Some(crate::virtual_clipboard::clipboard_sequence())
}

pub(in crate::app) fn clipboard_file_ops_supported() -> bool {
    true
}

pub(in crate::app) fn drag_out_files(files: &[String]) {
    crate::dragout::drag_out(files);
}

pub(in crate::app) fn os_drag_out_supported() -> bool {
    true
}

pub(in crate::app) fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(in crate::app) fn available_space_for_path(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new("."))
    };
    let wide = path_to_wide(dir);
    let mut free = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
    (ok != 0).then_some(free)
}

pub(in crate::app) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let src_w = path_to_wide(src);
    let dest_w = path_to_wide(dest);
    let ok = unsafe {
        MoveFileExW(
            src_w.as_ptr(),
            dest_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(in crate::app) fn process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

impl App {
    pub(in crate::app) fn initial_context_menu_enabled() -> bool {
        crate::shell_register::context_menu_enabled()
    }

    pub(in crate::app) fn should_start_watcher(&self) -> bool {
        self.watcher.is_none() && !self.folder_index.is_empty()
    }

    pub(in crate::app) fn should_start_clip_key_poller(&self) -> bool {
        self.clip_key_rx.is_none()
    }
}
