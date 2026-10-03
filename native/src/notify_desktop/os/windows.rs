//! A hidden window owns a Shell notification icon while its balloon lives.
use crate::syncjobs::ProblemNotice;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_TIP, NIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, DispatchMessageW, LoadIconW, PeekMessageW, TranslateMessage, IDI_APPLICATION, MSG, PM_REMOVE};
pub(super) fn notify(notice: &ProblemNotice) -> Result<(), String> {
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let window = unsafe { CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0,
        std::ptr::null_mut(), std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null()) };
    if window.is_null() { return Err(std::io::Error::last_os_error().to_string()); }
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of_val(&data) as u32;
    data.hWnd = window; data.uID = 1; data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
    data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
    text(&mut data.szTip, "Smart Explorer"); text(&mut data.szInfoTitle, &notice.title); text(&mut data.szInfo, &notice.text);
    data.dwInfoFlags = NIIF_WARNING;
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
    if added {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(30) {
            let mut message: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut message, window, 0, 0, PM_REMOVE) } != 0 {
                unsafe { TranslateMessage(&message); DispatchMessageW(&message); }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data); }
    }
    unsafe { DestroyWindow(window); }
    if added { Ok(()) } else { Err("Windows hat die Systembenachrichtigung nicht angenommen".into()) }
}
fn text<const N: usize>(buffer: &mut [u16; N], value: &str) {
    let mut at = 0;
    for character in value.chars().filter(|character| *character != '\0') {
        let mut encoded = [0; 2]; let units = character.encode_utf16(&mut encoded);
        if at + units.len() >= N { break; }
        buffer[at..at + units.len()].copy_from_slice(units); at += units.len();
    }
}
