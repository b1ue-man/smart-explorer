//! A password in the system's credential dialog
//! (`CredUIPromptForWindowsCredentialsW` with `CREDUIWIN_GENERIC`, unpacked
//! with `CredUnPackAuthenticationBufferW`). The buffers are wiped before they
//! are freed; nothing is saved (`pfSave` stays null).
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_CANCELLED;
use windows_sys::Win32::Security::Credentials::{
    CredUIPromptForWindowsCredentialsW, CredUnPackAuthenticationBufferW, CREDUIWIN_GENERIC,
    CREDUI_INFOW,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use zeroize::{Zeroize, Zeroizing};

use super::super::types::ElevateError;

const MAX_FIELD: usize = 512;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn ask_password(
    caption: &str,
    message: &str,
) -> Result<Option<Zeroizing<String>>, ElevateError> {
    let caption = wide(caption);
    let message = wide(message);
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: null_mut(),
        pszMessageText: message.as_ptr(),
        pszCaptionText: caption.as_ptr(),
        hbmBanner: null_mut(),
    };
    let mut package = 0u32;
    let mut out: *mut c_void = null_mut();
    let mut out_size = 0u32;
    // SAFETY: the info structure and out pointers live for the call.
    let status = unsafe {
        CredUIPromptForWindowsCredentialsW(
            &info,
            0,
            &mut package,
            null(),
            0,
            &mut out,
            &mut out_size,
            null_mut(),
            CREDUIWIN_GENERIC,
        )
    };
    if status == ERROR_CANCELLED {
        return Ok(None);
    }
    if status != 0 {
        return Err(ElevateError::Failed(format!(
            "the credential dialog failed: {}",
            std::io::Error::from_raw_os_error(status as i32)
        )));
    }
    let mut user = vec![0u16; MAX_FIELD];
    let mut domain = vec![0u16; MAX_FIELD];
    let mut password = Zeroizing::new(vec![0u16; MAX_FIELD]);
    let (mut user_len, mut domain_len, mut password_len) =
        (MAX_FIELD as u32, MAX_FIELD as u32, MAX_FIELD as u32);
    // SAFETY: the buffer from the dialog and three buffers of the sizes passed.
    let unpacked = unsafe {
        CredUnPackAuthenticationBufferW(
            0,
            out,
            out_size,
            user.as_mut_ptr(),
            &mut user_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            password.as_mut_ptr(),
            &mut password_len,
        )
    };
    // SAFETY: the dialog's buffer of `out_size` bytes, wiped and freed once.
    unsafe {
        std::slice::from_raw_parts_mut(out as *mut u8, out_size as usize).zeroize();
        CoTaskMemFree(out);
    }
    user.zeroize();
    domain.zeroize();
    if unpacked == 0 {
        return Err(ElevateError::Failed(format!(
            "the credential dialog's answer could not be read: {}",
            std::io::Error::last_os_error()
        )));
    }
    let length = password
        .iter()
        .take(password_len as usize)
        .position(|unit| *unit == 0)
        .unwrap_or((password_len as usize).min(MAX_FIELD));
    Ok(Some(Zeroizing::new(String::from_utf16_lossy(
        &password[..length],
    ))))
}
