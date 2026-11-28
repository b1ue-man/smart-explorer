//! The token of this process: whether it is elevated, and the user's SID as
//! SDDL text for the pipe ACL.
use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenUser, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn own_token() -> io::Result<Token> {
    let mut handle: HANDLE = null_mut();
    // SAFETY: the pseudo handle of this process and an out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Token(handle))
}

pub(super) fn is_elevated() -> bool {
    let Ok(token) = own_token() else {
        return false;
    };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut length = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// `S-1-5-21-…` of the user this process runs as.
pub(super) fn user_sid_text() -> io::Result<String> {
    let token = own_token()?;
    let mut length = 0u32;
    // SAFETY: a size query with no buffer.
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut length);
    }
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 words keep the TOKEN_USER aligned.
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    // SAFETY: the buffer holds `length` bytes.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr() as *mut c_void,
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetTokenInformation wrote a TOKEN_USER at the start.
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut text: *mut u16 = null_mut();
    // SAFETY: the SID points into `buffer`, which lives until the end.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a NUL-terminated string LocalAlloc'ed by the call.
    let sid = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text as *mut c_void);
        sid
    };
    Ok(sid)
}
