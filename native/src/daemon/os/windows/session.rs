//! Logon identity, stable across daemon crashes but changed by a new logon.
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Security::{GetTokenInformation, TokenStatistics, TOKEN_QUERY, TOKEN_STATISTICS};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
pub(crate) fn session_marker() -> Option<String> {
    if let Ok(marker) = std::env::var("SE_SYNC_SESSION") { return Some(marker); }
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 { return None; }
    let mut info: TOKEN_STATISTICS = unsafe { std::mem::zeroed() };
    let mut length = 0;
    let success = unsafe { GetTokenInformation(token, TokenStatistics,
        (&mut info as *mut TOKEN_STATISTICS).cast(), std::mem::size_of_val(&info) as u32, &mut length) };
    unsafe { CloseHandle(token); }
    (success != 0).then(|| format!("{:08x}{:08x}", info.AuthenticationId.HighPart, info.AuthenticationId.LowPart))
}
