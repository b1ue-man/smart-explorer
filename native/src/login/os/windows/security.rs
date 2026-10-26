//! SID, token and security-descriptor utilities; no process-wide impersonation.
use std::{ffi::c_void, io, ptr::{null, null_mut}};
use windows_sys::Win32::{Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree}, Security::{self, Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW}, GetTokenInformation, TokenUser, TOKEN_QUERY}};
use crate::login::os::shared::denied;
pub(super) fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }
pub(super) struct Handle(pub HANDLE);
unsafe impl Send for Handle {}
impl Handle { pub fn new(raw: HANDLE) -> io::Result<Self> { if raw.is_null() || raw == INVALID_HANDLE_VALUE { Err(io::Error::last_os_error()) } else { Ok(Self(raw)) } } }
impl Drop for Handle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
pub(super) fn sid_string(sid: *mut c_void) -> io::Result<String> {
    let mut text = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 { return Err(io::Error::last_os_error()); }
    let mut len = 0; unsafe { while *text.add(len) != 0 { len += 1; } }
    let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    unsafe { LocalFree(text.cast()); } Ok(result)
}
pub(super) fn token_data(token: HANDLE, class: Security::TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
    let mut len = 0u32; unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut len); }
    if len == 0 || len > 1024 * 1024 { return Err(denied("invalid Windows token information length")); }
    let mut data = vec![0usize; (len as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token, class, data.as_mut_ptr().cast(), len, &mut len) } == 0 { return Err(io::Error::last_os_error()); }
    Ok(data)
}
pub(super) fn token_user(token: HANDLE) -> io::Result<String> {
    let data = token_data(token, TokenUser)?;
    let user = unsafe { &*data.as_ptr().cast::<Security::TOKEN_USER>() }; sid_string(user.User.Sid)
}
pub(super) fn process_token(process: HANDLE) -> io::Result<Handle> {
    let mut token = null_mut();
    if unsafe { windows_sys::Win32::System::Threading::OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 { return Err(io::Error::last_os_error()); }
    Handle::new(token)
}
pub(super) fn require_system() -> io::Result<()> {
    let token = process_token(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })?;
    if token_user(token.0)? != "S-1-5-18" { return Err(denied("login broker/helper requires LocalSystem")); } Ok(())
}
pub(super) fn service_sid(name: &str) -> io::Result<String> {
    let account = wide(&format!("NT SERVICE\\{name}"));
    let mut sid_len = 0u32; let mut domain_len = 0u32; let mut kind = 0i32;
    unsafe { Security::LookupAccountNameW(null(), account.as_ptr(), null_mut(), &mut sid_len, null_mut(), &mut domain_len, &mut kind); }
    if sid_len == 0 || sid_len > 1024 || domain_len > 1024 { return Err(denied("gateway service SID is unavailable; administrative service installation is required")); }
    let mut sid = vec![0usize; (sid_len as usize).div_ceil(std::mem::size_of::<usize>())]; let mut domain = vec![0u16; domain_len as usize];
    if unsafe { Security::LookupAccountNameW(null(), account.as_ptr(), sid.as_mut_ptr().cast(), &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind) } == 0 { return Err(io::Error::last_os_error()); }
    sid_string(sid.as_mut_ptr().cast())
}
pub(super) struct Descriptor(*mut c_void);
impl Descriptor {
    pub fn new(sddl: &str) -> io::Result<Self> {
        let mut raw = null_mut();
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(sddl).as_ptr(), 1, &mut raw, null_mut()) } == 0 { return Err(io::Error::last_os_error()); }
        Ok(Self(raw))
    }
    pub fn attributes(&self) -> Security::SECURITY_ATTRIBUTES { Security::SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<Security::SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: self.0, bInheritHandle: 0 } }
}
impl Drop for Descriptor { fn drop(&mut self) { unsafe { LocalFree(self.0); } } }

pub(super) fn require_administrator() -> io::Result<()> {
    let descriptor = Descriptor::new("O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)")?;
    let mut owner = null_mut(); let mut defaulted = 0;
    if unsafe { Security::GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted) } == 0 { return Err(io::Error::last_os_error()); }
    let mut member = 0;
    if unsafe { Security::CheckTokenMembership(null_mut(), owner, &mut member) } == 0 { return Err(io::Error::last_os_error()); }
    if member == 0 { return Err(denied("login provisioning requires an elevated administrator")); } Ok(())
}
