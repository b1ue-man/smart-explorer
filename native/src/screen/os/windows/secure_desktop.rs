//! Secure desktop and session state of Windows (SCR-M4, decisions C1 and
//! U16-7). A UAC prompt and the lock screen run on a desktop of their own
//! (`Winlogon`) that a user process cannot open: capture pauses with the
//! state `secure_desktop` (the capture APIs give nothing usable there, A9).
//! A disconnected remote session pauses it with `session_inactive`.
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
    DESKTOP_READOBJECTS, UOI_NAME,
};

/// UTF-16 units read of a desktop name (`Default`, `Winlogon`, …).
const NAME_UNITS: usize = 256;

/// Whether `name` is the desktop of the user's programs.
pub(super) fn is_default_desktop(name: &str) -> bool {
    name.eq_ignore_ascii_case("Default")
}

/// Whether the input desktop is not the user's: opening it fails (the
/// secure desktop of a UAC prompt or the lock screen belongs to the system)
/// or its name is not `Default`. An unreadable desktop identity cannot
/// authorize capture or input and is treated as protected.
pub(super) fn on_secure_desktop() -> bool {
    // SAFETY: no handle is inherited; the access right only reads objects.
    let desktop =
        match unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) } {
            Ok(desktop) => desktop,
            Err(_) => return true,
        };
    let mut name = [0u16; NAME_UNITS];
    let mut needed = 0u32;
    // SAFETY: a desktop handle is a user object handle; the buffer holds
    // `NAME_UNITS` units and the length passed is its size in bytes.
    let read = unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast::<std::ffi::c_void>()),
            (NAME_UNITS * 2) as u32,
            Some(&mut needed),
        )
    };
    // SAFETY: the handle was opened above and is closed once.
    let _ = unsafe { CloseDesktop(desktop) };
    match read {
        Ok(()) => {
            let end = name
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(NAME_UNITS);
            !is_default_desktop(&String::from_utf16_lossy(&name[..end]))
        }
        Err(_) => true,
    }
}

/// Whether the session of this process is connected (`WTSActive`). Any
/// other connect state, for example a disconnected remote session, is not;
/// a failing or truncated query cannot authorize the process session.
pub(super) fn session_active() -> bool {
    use windows_sys::Win32::System::RemoteDesktop::{
        WTSActive, WTSConnectState, WTSFreeMemory, WTSQuerySessionInformationW,
        WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION,
    };
    let mut buffer: *mut u16 = std::ptr::null_mut();
    let mut bytes = 0u32;
    // SAFETY: the out pointers address locals; the buffer the call returns
    // is released with `WTSFreeMemory` below.
    let queried = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            WTSConnectState,
            &mut buffer,
            &mut bytes,
        )
    };
    if queried == 0 || buffer.is_null() {
        return false;
    }
    let state = if bytes as usize >= std::mem::size_of::<i32>() {
        // SAFETY: the connect state is one `WTS_CONNECTSTATE_CLASS` (an
        // int) in a buffer of at least that many bytes.
        unsafe { buffer.cast::<i32>().read_unaligned() }
    } else {
        -1
    };
    // SAFETY: the buffer came from the query above and is freed once.
    unsafe { WTSFreeMemory(buffer.cast()) };
    state == WTSActive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_desktop_names_compare_without_case() {
        assert!(is_default_desktop("Default"));
        assert!(is_default_desktop("default"));
        assert!(!is_default_desktop("Winlogon"));
        assert!(!is_default_desktop(""));
    }
}
