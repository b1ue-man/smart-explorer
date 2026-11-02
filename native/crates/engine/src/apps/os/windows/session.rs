//! A per-user interactive adapter, never an Exec service/session-0 launcher.
use crate::apps::core::model::{HostSession, SessionId, UserId};
use std::{
    io,
    mem::{size_of, MaybeUninit},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authentication::Identity::*, Authorization::*, *},
    System::{RemoteDesktop::*, Threading::*},
    UI::WindowsAndMessaging::GetShellWindow,
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn refused(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}
fn process_token() -> io::Result<Token> {
    let mut thread = ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut thread) } != 0 {
        unsafe {
            CloseHandle(thread);
        }
        return Err(refused(
            "impersonated worker cannot launch GUI applications",
        ));
    }
    if unsafe { GetLastError() } != ERROR_NO_TOKEN {
        return Err(io::Error::last_os_error());
    }
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Token(token))
}
fn statistics(token: &Token) -> io::Result<TOKEN_STATISTICS> {
    let mut data = MaybeUninit::<TOKEN_STATISTICS>::uninit();
    let mut size = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenStatistics,
            data.as_mut_ptr().cast(),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { data.assume_init() })
}
pub(super) fn sid() -> io::Result<String> {
    let token = process_token()?;
    token_sid(&token)
}
fn token_sid(token: &Token) -> io::Result<String> {
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 64 * 1024 {
        return Err(io::Error::last_os_error());
    }
    // u64 storage guarantees TOKEN_USER pointer alignment.
    let mut data = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*data.as_ptr().cast::<TOKEN_USER>() };
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    unsafe {
        LocalFree(text.cast());
    }
    Ok(result)
}
pub(super) fn matches_process(process: HANDLE, expected: &HostSession) -> io::Result<bool> {
    let current = current()?;
    if current.user_id != expected.user_id || current.session_id != expected.session_id {
        return Ok(false);
    }
    let mut handle = ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Token(handle);
    let actual = statistics(&token)?.AuthenticationId;
    let own = statistics(&process_token()?)?.AuthenticationId;
    Ok(actual.HighPart == own.HighPart
        && actual.LowPart == own.LowPart
        && expected.user_id.as_str() == format!("windows-sid:{}", token_sid(&token)?))
}
pub(super) fn current() -> io::Result<HostSession> {
    let token = process_token()?;
    let stats = statistics(&token)?;
    let mut session = 0u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if session == 0 || unsafe { GetShellWindow() }.is_null() {
        return Err(refused("GUI launch requires the actual interactive user's desktop, not session 0 or an Exec service"));
    }
    let mut data = ptr::null_mut();
    if unsafe { LsaGetLogonSessionData(&stats.AuthenticationId, &mut data) } != 0 || data.is_null()
    {
        return Err(refused("interactive logon identity is unavailable"));
    }
    let logon = unsafe { &*data };
    let logon_time = logon.LogonTime;
    let valid =
        logon.Session == session && matches!(logon.LogonType, 2 | 10 | 11 | 12) && logon_time > 0;
    unsafe {
        LsaFreeReturnBuffer(data.cast());
    }
    if !valid {
        return Err(refused(
            "process token does not belong to this interactive logon",
        ));
    }
    let mut info = ptr::null_mut();
    let mut bytes = 0;
    if unsafe {
        WTSQuerySessionInformationW(
            ptr::null_mut(),
            session,
            WTSSessionInfoEx,
            &mut info,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let facts = if bytes as usize >= size_of::<WTSINFOEXW>() {
        let item = unsafe { &*info.cast::<WTSINFOEXW>() };
        (item.Level == 1).then(|| unsafe { item.Data.WTSInfoExLevel1 })
    } else {
        None
    };
    unsafe {
        WTSFreeMemory(info.cast());
    }
    let facts = facts.ok_or_else(|| refused("current WTS login facts are unavailable"))?;
    if facts.SessionId != session || facts.SessionState != WTSActive || facts.LogonTime <= 0 {
        return Err(refused(
            "interactive session is disconnected, changed or logged out",
        ));
    }
    Ok(HostSession {
        user_id: UserId::try_from(format!("windows-sid:{}", sid()?))?,
        session_id: SessionId::try_from(format!(
            "windows-login:{session}:{}:{}:{logon_time}:{}",
            stats.AuthenticationId.HighPart, stats.AuthenticationId.LowPart, facts.LogonTime
        ))?,
        interactive: true,
        unlocked: facts.SessionFlags == WTS_SESSIONSTATE_UNLOCK as i32,
    })
}
