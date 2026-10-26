//! Cross-session CreateProcessAsUser never inherits anonymous pipe handles.
use super::{
    pipe,
    security::{self, wide, Handle},
};
use crate::login::{
    core::model::{SessionBinding, HELPER_SWITCH, PROBE_SWITCH},
    os::shared::{
        child::{Lifetime, Spawned},
        denied,
        installation::{self, Installation},
        nonce,
    },
};
use std::{
    io,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, WAIT_OBJECT_0},
    Security::{
        self, DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TokenSessionId,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_ALL_ACCESS, TOKEN_DUPLICATE, TOKEN_QUERY,
    },
    System::{JobObjects::*, Threading::*},
};
struct Life {
    process: Handle,
    _job: Handle,
}
impl Drop for Life {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
impl Lifetime for Life {
    fn process_id(&self) -> u32 {
        unsafe { GetProcessId(self.process.0) }
    }
    fn stop(&mut self) -> bool {
        if unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_OBJECT_0 {
            return true;
        }
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if unsafe { WaitForSingleObject(self.process.0, 20) } == WAIT_OBJECT_0 {
                return true;
            }
        }
        // TerminateProcess is asynchronous. Only the process handle's signaled
        // state confirms exit, including when termination itself returned 0.
        unsafe {
            TerminateProcess(self.process.0, 1);
            WaitForSingleObject(self.process.0, 2000) == WAIT_OBJECT_0
        }
    }
}
pub(super) fn console_token(session: u32) -> io::Result<Handle> {
    security::require_system()?;
    let mut own = null_mut();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
            &mut own,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let own = Handle::new(own)?;
    enable(own.0, "SeTcbPrivilege")?;
    let mut raw = null_mut();
    if unsafe {
        DuplicateTokenEx(
            own.0,
            TOKEN_ALL_ACCESS,
            null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut raw,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token = Handle::new(raw)?;
    if unsafe {
        Security::SetTokenInformation(
            token.0,
            TokenSessionId,
            (&session as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(token)
}
fn enable(token: HANDLE, name: &str) -> io::Result<()> {
    let mut luid = unsafe { std::mem::zeroed() };
    if unsafe { Security::LookupPrivilegeValueW(null(), wide(name).as_ptr(), &mut luid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let privileges = Security::TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [Security::LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: Security::SE_PRIVILEGE_ENABLED,
        }],
    };
    unsafe {
        windows_sys::Win32::Foundation::SetLastError(0);
    }
    if unsafe { Security::AdjustTokenPrivileges(token, 0, &privileges, 0, null_mut(), null_mut()) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(0) {
        return Err(error);
    }
    Ok(())
}
pub(crate) fn spawn(
    install: &Installation,
    binding: &SessionBinding,
    probe: bool,
) -> io::Result<Spawned> {
    let session: u32 = binding.session_id.parse().map_err(io::Error::other)?;
    if unsafe { windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() }
        != session
    {
        return Err(denied("login target is not the active console session"));
    }
    let token = console_token(session)?;
    let channel = nonce()?;
    let server = pipe::create(
        &format!(r"\\.\pipe\SmartExplorerLoginHelper-{channel}"),
        None,
    )?;
    let image = wide(&install.image_path.to_string_lossy());
    let mut command = wide(&format!(
        "\"{}\" {}",
        install.image_path.display(),
        if probe { PROBE_SWITCH } else { HELPER_SWITCH }
    ));
    let mut desktop = wide("winsta0\\default");
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop.as_mut_ptr();
    let system_root = super::session::windows_directory()?;
    let environment = format!("SE_LOGIN_HELPER_CHANNEL={channel}\0SE_LOGIN_HELPER_PARENT={}\0SystemRoot={}\0WINDIR={}\0\0", std::process::id(), system_root.display(), system_root.display());
    let environment: Vec<u16> = environment.encode_utf16().collect();
    let current = wide(
        &install
            .image_path
            .parent()
            .ok_or_else(|| denied("login image has no protected parent"))?
            .to_string_lossy(),
    );
    let job = Handle::new(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limit: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limit.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limit as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limit) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // Held no-write/no-delete file and parent pins survive until this call returns.
    installation::verify_hash(&install.image.file, &install.manifest.sha256)?;
    if unsafe {
        CreateProcessAsUserW(
            token.0,
            image.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            current.as_ptr(),
            &startup,
            &mut information,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let process = Handle::new(information.hProcess)?;
    let mut life = Life { process, _job: job };
    let thread = Handle::new(information.hThread)?;
    if unsafe { AssignProcessToJobObject(life._job.0, life.process.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        life.stop();
        return Err(io::Error::last_os_error());
    }
    if let Err(error) = pipe::accept(&server, Duration::from_secs(5), || false) {
        life.stop();
        return Err(error);
    }
    if pipe::client_pid(&server)? != information.dwProcessId {
        life.stop();
        return Err(denied("login helper pipe was opened by the wrong process"));
    }
    super::session::verify_system_process(information.dwProcessId)?;
    Ok(Spawned {
        input: Box::new(server.clone_stream()?),
        output: Box::new(server),
        life: Box::new(life),
    })
}
