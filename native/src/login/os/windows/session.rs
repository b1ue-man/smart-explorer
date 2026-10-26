//! Active console, actual user token and fresh input-desktop attachment.
use super::security::{self, Handle};
use crate::login::{
    core::model::{DesktopKind, SessionBinding},
    os::shared::{
        child::HelperLink,
        denied,
        installation::{self, Installation},
    },
};
use std::{
    cell::RefCell, ffi::OsString, io, os::windows::ffi::OsStringExt, path::PathBuf, ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Security::{TokenStatistics, TOKEN_STATISTICS},
    System::{RemoteDesktop::*, StationsAndDesktops::*, Threading::*},
};
thread_local! { static DESKTOP: RefCell<Option<HDESK>> = const { RefCell::new(None) }; }
pub(super) fn windows_directory() -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let len = unsafe {
        windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if len == 0 || len >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len])))
}
pub(super) fn verify_system_process(pid: u32) -> io::Result<()> {
    let process = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
    let token = security::process_token(process.0)?;
    if security::token_user(token.0)? != "S-1-5-18" {
        return Err(denied("login channel process is not LocalSystem"));
    }
    let mut text = vec![0u16; 32768];
    let mut len = text.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.0, 0, text.as_mut_ptr(), &mut len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let actual = PathBuf::from(OsString::from_wide(&text[..len as usize]));
    let install = installation::load()?;
    if !same_file::is_same_file(actual, &install.image_path)? {
        return Err(denied(
            "login channel process is not the administrative release image",
        ));
    }
    installation::verify_hash(&install.image.file, &install.manifest.sha256)
}
fn base() -> io::Result<(SessionBinding, bool)> {
    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == u32::MAX {
        return Err(denied("Windows has no active console session"));
    }
    let mut buffer = null_mut();
    let mut bytes = 0;
    if unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if buffer.is_null() || (bytes as usize) < std::mem::size_of::<WTSINFOEXW>() {
        if !buffer.is_null() {
            unsafe {
                WTSFreeMemory(buffer.cast());
            }
        }
        return Err(denied("Windows console state has an invalid size"));
    }
    let data = unsafe { buffer.cast::<WTSINFOEXW>().read_unaligned() };
    unsafe {
        WTSFreeMemory(buffer.cast());
    }
    if data.Level != 1 {
        return Err(denied("unsupported Windows console state level"));
    }
    let info = unsafe { data.Data.WTSInfoExLevel1 };
    if info.SessionId != session || info.SessionState != WTSActive {
        return Err(denied("Windows console is not active"));
    }
    let mut authentication = "before-login".to_string();
    let user = if info.UserName.iter().all(|unit| *unit == 0) {
        None
    } else {
        let mut raw = null_mut();
        if unsafe { WTSQueryUserToken(session, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle::new(raw)?;
        let stats = security::token_data(token.0, TokenStatistics)?;
        let stats = unsafe { &*stats.as_ptr().cast::<TOKEN_STATISTICS>() };
        authentication = format!(
            "{}:{}",
            stats.AuthenticationId.HighPart, stats.AuthenticationId.LowPart
        );
        Some(security::token_user(token.0)?)
    };
    let generation = format!("{session}:{}:{authentication}", info.LogonTime);
    // SessionFlags are documented for Windows 8+, the project's supported hosts.
    let locked = if user.is_none() {
        true
    } else {
        match info.SessionFlags {
            0 => true,
            1 => false,
            _ => return Err(denied("Windows console lock state is unavailable")),
        }
    };
    Ok((
        SessionBinding {
            session_id: session.to_string(),
            user_id: user,
            session_generation: generation,
            desktop_generation: String::new(),
            desktop: DesktopKind::Unavailable,
        },
        locked,
    ))
}
struct Desktop(HDESK);
impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            CloseDesktop(self.0);
        }
    }
}
fn input_desktop() -> io::Result<(Desktop, String)> {
    let handle = unsafe {
        OpenInputDesktop(
            0,
            0,
            DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS | DESKTOP_SWITCHDESKTOP,
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let desktop = Desktop(handle);
    let mut units = vec![0u16; 256];
    let mut needed = 0;
    if unsafe {
        GetUserObjectInformationW(
            handle as HANDLE,
            UOI_NAME,
            units.as_mut_ptr().cast(),
            (units.len() * 2) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| denied("Windows input desktop name is not terminated"))?;
    Ok((desktop, String::from_utf16_lossy(&units[..end])))
}
fn observed() -> io::Result<(SessionBinding, Desktop)> {
    let (mut binding, locked) = base()?;
    let (desktop, name) = input_desktop()?;
    binding.desktop = if name.eq_ignore_ascii_case("Default") {
        DesktopKind::UserSession
    } else if !name.eq_ignore_ascii_case("Winlogon") {
        DesktopKind::Unavailable
    } else if binding.user_id.is_none() {
        DesktopKind::BeforeLogin
    } else if locked {
        DesktopKind::Locked
    } else {
        DesktopKind::Uac
    };
    binding.desktop_generation = format!(
        "{}:{}:{locked}",
        binding.session_generation,
        name.to_ascii_lowercase()
    );
    Ok((binding, desktop))
}
struct ProbeCache {
    session: String,
    link: HelperLink,
}
static PROBE: std::sync::Mutex<Option<ProbeCache>> = std::sync::Mutex::new(None);
pub(crate) fn binding(install: &Installation) -> io::Result<SessionBinding> {
    let (base, _) = base()?;
    let identity = format!(
        "{}:{}",
        base.session_generation, install.manifest.generation
    );
    let mut slot = PROBE
        .lock()
        .map_err(|_| denied("console probe state is unavailable"))?;
    if slot.as_ref().is_some_and(|cache| cache.session != identity) {
        slot.take();
    }
    for _ in 0..2 {
        if let Some(cache) = slot.as_mut() {
            match cache.link.observe().and_then(|reply| {
                reply
                    .binding
                    .ok_or_else(|| denied("console probe returned no binding"))
            }) {
                Ok(binding) => return Ok(binding),
                Err(_) => {
                    slot.take();
                }
            }
        } else {
            let mut link = HelperLink::start(install, &base, true)?;
            let reply = link.ready()?;
            let binding = reply
                .binding
                .ok_or_else(|| denied("console probe returned no binding"))?;
            *slot = Some(ProbeCache {
                session: identity.clone(),
                link,
            });
            return Ok(binding);
        }
    }
    Err(denied("console desktop probe ended"))
}
fn matches_observed(actual: &SessionBinding, expected: &SessionBinding) -> bool {
    actual.session_id == expected.session_id
        && actual.user_id == expected.user_id
        && actual.session_generation == expected.session_generation
        && actual.desktop == expected.desktop
        && expected.desktop_generation.split('|').next() == Some(actual.desktop_generation.as_str())
}
pub(crate) fn run_probe(
    mut input: Box<dyn std::io::Read + Send>,
    mut output: Box<dyn std::io::Write + Send>,
    init: crate::login::core::wire::HelperInit,
    first: SessionBinding,
) -> io::Result<()> {
    use crate::login::core::wire::{self, HelperCall, HelperCommand, Reply};
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("login-desktop-probe-input".into())
        .spawn(move || loop {
            let call = wire::receive::<HelperCall>(&mut input);
            let end = call.is_err();
            if sender.send(call).is_err() || end {
                break;
            }
        })?;
    let mut raw = first;
    let mut generation = crate::login::os::shared::nonce()?;
    let reply_for = |raw: &SessionBinding, generation: &str| {
        let mut binding = raw.clone();
        binding.desktop_generation = format!("{}|{generation}", binding.desktop_generation);
        let mut reply = Reply::state("probed");
        reply.binding = Some(binding);
        reply
    };
    wire::send(&mut output, &reply_for(&raw, &generation))?;
    loop {
        let (latest, _) = observed()?;
        if latest != raw {
            raw = latest;
            generation = crate::login::os::shared::nonce()?;
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(call) => {
                let call = call?;
                if call.cookie != init.cookie {
                    return Err(denied("wrong console probe capability"));
                }
                match call.command {
                    HelperCommand::Observe => {
                        wire::send(&mut output, &reply_for(&raw, &generation))?
                    }
                    HelperCommand::Operation(crate::login::core::model::Operation::Stop) => {
                        return Ok(())
                    }
                    _ => return Err(denied("metadata probe cannot capture or inject input")),
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Ok(()),
        }
    }
}
fn parent_process_id() -> io::Result<u32> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while found {
        if entry.th32ProcessID == std::process::id() {
            return Ok(entry.th32ParentProcessID);
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    Err(denied("Windows helper parent process is unavailable"))
}
pub(crate) fn prepare_helper(
    expected: &SessionBinding,
    parent: u32,
    probe: bool,
) -> io::Result<SessionBinding> {
    security::require_system()?;
    if parent_process_id()? != parent {
        return Err(denied(
            "Windows helper actual parent does not match its broker channel",
        ));
    }
    verify_system_process(parent)?;
    let mut own_session = 0u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut own_session) } == 0
        || expected.session_id != own_session.to_string()
        || own_session != unsafe { WTSGetActiveConsoleSessionId() }
    {
        return Err(denied("helper is outside the active console session"));
    }
    let (actual, desktop) = observed()?;
    if !probe {
        if !matches_observed(&actual, expected) {
            return Err(denied("actual input desktop differs from signed binding"));
        }
        if unsafe { SetThreadDesktop(desktop.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        DESKTOP.with(|slot| *slot.borrow_mut() = Some(desktop.0));
        // The fresh process owns this desktop pin until exit. Closing a desktop
        // still assigned to a thread is invalid; the OS closes it at process exit.
        std::mem::forget(desktop);
    }
    Ok(if probe { actual } else { expected.clone() })
}
pub(crate) fn helper_binding_matches(expected: &SessionBinding) -> io::Result<bool> {
    let attached_is_input = DESKTOP.with(|slot| {
        let Some(desktop) = *slot.borrow() else {
            return false;
        };
        let mut active = 0i32;
        let mut needed = 0;
        unsafe {
            GetUserObjectInformationW(
                desktop as HANDLE,
                UOI_IO,
                (&mut active as *mut i32).cast(),
                std::mem::size_of::<i32>() as u32,
                &mut needed,
            ) != 0
                && active != 0
        }
    });
    if !attached_is_input {
        return Ok(false);
    }
    observed().map(|(actual, _)| matches_observed(&actual, expected))
}
pub(crate) fn finish_helper() {}
