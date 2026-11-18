//! The input desktop of the thread that duplicates (plan 2026-11-17 P9).
//!
//! `DuplicateOutput` duplicates the desktop of the calling thread's desktop
//! only while that desktop is the input desktop. A process that runs as
//! LOCAL_SYSTEM in the user's window station (the verified login helper,
//! `login_source.rs`) or a thread that was started while another desktop was
//! the input one keeps its old desktop after a switch (UAC prompt, lock
//! screen and back) and gets `E_ACCESSDENIED` or `DXGI_ERROR_ACCESS_LOST`
//! for every new duplication. Sunshine therefore puts its capture thread on
//! the input desktop before each duplication (`syncThreadDesktop` in
//! `src/platform/windows/misc.cpp`: `OpenInputDesktop` with
//! `GENERIC_ALL`, `SetThreadDesktop`, `CloseDesktop`). This does the same,
//! and only when the names differ, so an ordinary user process, whose thread
//! is on `Default` and who cannot open the secure desktop, is not touched.
use windows::Win32::Foundation::{GENERIC_ALL, HANDLE};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop,
    DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, HDESK, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

/// UTF-16 units read of a desktop name.
const NAME_UNITS: usize = 256;

fn name_of(desktop: HDESK) -> Option<String> {
    let mut name = [0u16; NAME_UNITS];
    let mut needed = 0u32;
    // SAFETY: a desktop handle is a user object handle; the buffer holds
    // `NAME_UNITS` units and the length passed is its size in bytes.
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast::<std::ffi::c_void>()),
            (NAME_UNITS * 2) as u32,
            Some(&mut needed),
        )
    }
    .ok()?;
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(NAME_UNITS);
    Some(String::from_utf16_lossy(&name[..end]))
}

/// Puts the calling thread on the input desktop when it is on another one;
/// a refusal goes to the host log and the duplication reports its own result.
pub(super) fn attach() {
    // SAFETY: no handle is inherited; the handle is closed below.
    let Ok(input) = (unsafe {
        OpenInputDesktop(
            DESKTOP_CONTROL_FLAGS(0),
            false,
            DESKTOP_ACCESS_FLAGS(GENERIC_ALL.0),
        )
    }) else {
        // The secure desktop of a user process: capture pauses there.
        return;
    };
    let wanted = name_of(input);
    // SAFETY: the desktop of this thread; such a handle is not closed.
    let current = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .ok()
        .and_then(name_of);
    if wanted.is_some() && wanted != current {
        // SAFETY: a live desktop handle of this window station.
        match unsafe { SetThreadDesktop(input) } {
            Ok(()) => eprintln!(
                "screen capture: desktop_attach: thread moved from {} to {}",
                current.as_deref().unwrap_or("?"),
                wanted.as_deref().unwrap_or("?")
            ),
            Err(error) => eprintln!(
                "screen capture: desktop_attach 0x{:08X} failed: {error}",
                error.code().0 as u32
            ),
        }
    }
    // SAFETY: the handle opened above, closed once (a desktop the thread now
    // uses stays open for it).
    let _ = unsafe { CloseDesktop(input) };
}
