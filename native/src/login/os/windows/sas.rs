//! Read-only Windows software-SAS policy; no global policy is ever written.
use super::{
    security::{self, wide},
    spawn,
};
use crate::login::{core::model::SessionBinding, os::shared::denied};
use std::io;
use windows_sys::Win32::{
    Foundation::FreeLibrary,
    Security,
    System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
};
pub(crate) fn secure_attention(binding: &SessionBinding) -> io::Result<()> {
    security::require_system()?;
    let policy = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            winreg::enums::KEY_READ | winreg::enums::KEY_WOW64_64KEY,
        )
        .and_then(|key| key.get_value::<u32, _>("SoftwareSASGeneration"));
    if !matches!(policy, Ok(1 | 3)) {
        return Err(denied("Windows software secure-attention policy does not permit services; it is not changed automatically"));
    }
    let session: u32 = binding.session_id.parse().map_err(io::Error::other)?;
    if unsafe { windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() }
        != session
    {
        return Err(denied("secure attention target is not the active console"));
    }
    let token = spawn::console_token(session)?;
    let library = unsafe {
        LoadLibraryExW(
            wide("sas.dll").as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if library.is_null() {
        return Err(io::Error::last_os_error());
    }
    struct Library(windows_sys::Win32::Foundation::HMODULE);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    let _library = Library(library);
    let function = unsafe { GetProcAddress(library, c"SendSAS".as_ptr().cast()) }
        .ok_or_else(|| io::Error::other("Windows SendSAS entrypoint is unavailable"))?;
    if unsafe { Security::ImpersonateLoggedOnUser(token.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            unsafe {
                Security::RevertToSelf();
            }
        }
    }
    let _revert = Revert;
    // Microsoft documents session-directed SAS through service impersonation.
    let send: unsafe extern "system" fn(i32) = unsafe { std::mem::transmute(function) };
    unsafe {
        send(0);
    } // SendSAS has no return value: report requested, never proven delivered.
    Ok(())
}
