//! One session of the Windows account service (C2, WE-2): the relay
//! (`se --share-exec-supervisor`, unchanged) runs as the session's account
//! in session 0, in a Job of the service that ends every process of the
//! session when the service closes it, stops it or ends itself. The relay's
//! standard input and output are the worker's two job pipes, which the
//! service opens only after checking that they are exactly the pipes of the
//! calling user; standard error goes to `NUL`. The relay gets the account's
//! profile (loaded with `SeBackupPrivilege` and `SeRestorePrivilege`), its
//! environment (`CreateEnvironmentBlock`) and its profile folder as working
//! folder, and is always the service's own program, checked against the
//! SHA-256 of the image the service started with.
use std::io;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::SE_KERNEL_OBJECT;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileType, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_PIPE, OPEN_EXISTING,
    SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows_sys::Win32::UI::Shell::{
    GetUserProfileDirectoryW, LoadUserProfileW, UnloadUserProfile, PROFILEINFOW,
};

use crate::share::exec_helper_protocol::HelperErrorCode;
use crate::share::exec_types::ExecId;

use super::exec_account_files::sha256_file;
use super::exec_account_pipes::job_pipe_id;
use super::exec_account_s4u::{session_token, CheckedAccount, Lsa};
use super::exec_account_security::{owner_of, wide, Sid};
use super::exec_account_service::lock;
use super::exec_handles::{AttributeList, OwnedHandle};
use super::exec_job_object::{active_processes, create_job, terminate_job};

/// `PI_NOUI` of `PROFILEINFOW`: no message box in session 0.
const PI_NOUI: u32 = 1;
/// The relay's mode: the unchanged supervisor of `exec_supervisor.rs`.
const RELAY_MODE: &str = "--share-exec-supervisor";
/// How long closing a session waits for its Job to be empty.
const CLOSE_WAIT: Duration = Duration::from_secs(10);

type Refusal = (HelperErrorCode, String);

/// SHA-256 of this service's program when it started.
static IMAGE: OnceLock<(PathBuf, String)> = OnceLock::new();

/// Remembers the program file and its SHA-256 at the service's start.
pub(super) fn remember_image() -> io::Result<()> {
    let path = std::env::current_exe()?;
    let hash = sha256_file(&path)?;
    let _ = IMAGE.set((path, hash));
    Ok(())
}

/// The relay's program: the service's own file, still the image it
/// started with. A changed file means an update came after the start.
fn relay_program() -> io::Result<PathBuf> {
    let (path, hash) = IMAGE
        .get()
        .ok_or_else(|| io::Error::other("the account service did not record its program"))?;
    if sha256_file(path)? != *hash {
        return Err(io::Error::other(format!(
            "{} differs from the running account service; update the account service",
            path.display()
        )));
    }
    Ok(path.clone())
}

/// A loaded profile, unloaded when the session closes.
struct Profile {
    token: OwnedHandle,
    handle: HANDLE,
}

/// One running session.
pub(super) struct Session {
    id: String,
    account: String,
    job: OwnedHandle,
    process: OwnedHandle,
    profile: Option<Profile>,
}

impl Session {
    /// Starts the relay as `account` with the caller's job pipes. The
    /// registration `lsa` is locked only for the logon.
    pub(super) fn start(
        lsa: &Mutex<Lsa>,
        caller: &Sid,
        account: &CheckedAccount,
        pipe_in: Option<&str>,
        pipe_out: Option<&str>,
    ) -> Result<Self, Refusal> {
        let (input, output) = open_job_pipes(caller, pipe_in, pipe_out)?;
        let program = relay_program().map_err(|error| outdated(&error))?;
        let id = ExecId::generate().map_err(failed)?.as_str().to_string();
        let token = session_token(&lock(lsa), account)?;
        let mut profile_handle: HANDLE = null_mut();
        load_profile(&token, &account.name, &mut profile_handle).map_err(failed)?;
        let profile = Profile {
            token,
            handle: profile_handle,
        };
        match launch(&profile.token, &program, &input, &output) {
            Ok((job, process)) => Ok(Self {
                id,
                account: account.name.clone(),
                job,
                process,
                profile: Some(profile),
            }),
            Err(error) => {
                unload(profile);
                Err(failed(error))
            }
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn account(&self) -> &str {
        &self.account
    }

    /// Ends every process of the session.
    pub(super) fn stop(&self) -> io::Result<()> {
        terminate_job(self.job.raw())
    }

    /// `(active, empty)`: the relay still runs; the Job has no process.
    pub(super) fn status(&self) -> io::Result<(bool, bool)> {
        let active = unsafe { WaitForSingleObject(self.process.raw(), 0) } == WAIT_TIMEOUT;
        Ok((active, active_processes(self.job.raw())? == 0))
    }

    /// Unloads the profile of a session whose processes all ended; the
    /// session stays listed for later status requests.
    pub(super) fn release_profile(&mut self) {
        if let Some(profile) = self.profile.take() {
            unload(profile);
        }
    }

    /// Ends the session: its processes, then its profile. Returns whether
    /// the Job became empty within `CLOSE_WAIT`.
    pub(super) fn close(mut self) -> bool {
        let _ = self.stop();
        let deadline = Instant::now() + CLOSE_WAIT;
        let mut empty = false;
        while Instant::now() < deadline {
            if active_processes(self.job.raw()).is_ok_and(|count| count == 0) {
                empty = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.release_profile();
        empty
    }
}

/// The caller's two job pipes, opened for the relay: exactly the names of
/// one session id, both pipes owned by the caller. The service does not
/// let the worker impersonate it (`SECURITY_ANONYMOUS`).
fn open_job_pipes(
    caller: &Sid,
    pipe_in: Option<&str>,
    pipe_out: Option<&str>,
) -> Result<(OwnedHandle, OwnedHandle), Refusal> {
    let (Some(pipe_in), Some(pipe_out)) = (pipe_in, pipe_out) else {
        return Err(bad("a start on Windows names the two pipes of the job"));
    };
    let ids = (job_pipe_id(pipe_in, "in"), job_pipe_id(pipe_out, "out"));
    let (Some(id_in), Some(id_out)) = ids else {
        return Err(bad("the pipes of a job are not Smart Explorer job pipes"));
    };
    if id_in != id_out {
        return Err(bad("the two pipes belong to different jobs"));
    }
    let input = open_pipe(pipe_in, GENERIC_READ, caller)?;
    let output = open_pipe(pipe_out, GENERIC_WRITE, caller)?;
    Ok((input, output))
}

fn open_pipe(name: &str, access: u32, caller: &Sid) -> Result<OwnedHandle, Refusal> {
    let wide_name = wide(name);
    let handle = unsafe {
        CreateFileW(
            wide_name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return Err(bad(&format!(
            "the job pipe {name} cannot be opened: {error}"
        )));
    }
    let handle = OwnedHandle::new(handle).map_err(failed)?;
    if unsafe { GetFileType(handle.raw()) } != FILE_TYPE_PIPE {
        return Err(bad(&format!("{name} is no pipe")));
    }
    let owner = owner_of(handle.raw(), SE_KERNEL_OBJECT).map_err(failed)?;
    if owner != *caller {
        return Err((
            HelperErrorCode::NotAllowed,
            format!("the job pipe {name} belongs to another user than the caller"),
        ));
    }
    Ok(handle)
}

/// Loads the profile of `name` (creating it on the first logon, A8).
fn load_profile(token: &OwnedHandle, name: &str, handle: &mut HANDLE) -> io::Result<()> {
    let mut user = wide(name);
    let mut info: PROFILEINFOW = unsafe { zeroed() };
    info.dwSize = size_of::<PROFILEINFOW>() as u32;
    info.dwFlags = PI_NOUI;
    info.lpUserName = user.as_mut_ptr();
    if unsafe { LoadUserProfileW(token.raw(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    *handle = info.hProfile;
    Ok(())
}

/// Unloads a profile; Windows keeps it loaded while handles of it are
/// open, the service logs that and carries on (C2).
fn unload(profile: Profile) {
    if unsafe { UnloadUserProfile(profile.token.raw(), profile.handle) } == 0 {
        let error = io::Error::last_os_error();
        super::exec_account_files::append_log(&format!("profile not unloaded yet: {error}"));
    }
}

fn profile_directory(token: &OwnedHandle) -> io::Result<Vec<u16>> {
    let mut length = 0u32;
    unsafe { GetUserProfileDirectoryW(token.raw(), null_mut(), &mut length) };
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; length as usize];
    if unsafe { GetUserProfileDirectoryW(token.raw(), buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

/// The account's environment block, destroyed on drop.
struct Environment(*mut core::ffi::c_void);

impl Environment {
    fn of(token: &OwnedHandle) -> io::Result<Self> {
        let mut block = null_mut();
        if unsafe { CreateEnvironmentBlock(&mut block, token.raw(), 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(block))
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        unsafe { DestroyEnvironmentBlock(self.0) };
    }
}

/// Starts the relay in a new Job; returns the Job and the relay process.
fn launch(
    token: &OwnedHandle,
    program: &Path,
    input: &OwnedHandle,
    output: &OwnedHandle,
) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let job = create_job()?;
    let environment = Environment::of(token)?;
    let directory = profile_directory(token)?;
    let null_device = null_output()?;
    let inherited = [input.raw(), output.raw(), null_device.raw()];
    for handle in inherited {
        inheritable(handle)?;
    }
    let mut attributes = AttributeList::new(2)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        inherited.as_ptr().cast(),
        size_of_val(&inherited),
    )?;
    let job_handle = job.raw();
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job_handle as *const HANDLE).cast(),
        size_of::<HANDLE>(),
    )?;
    let application = wide(program);
    let mut command_line: Vec<u16> = format!("se {RELAY_MODE}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // An empty desktop name: Windows connects the relay to a window
    // station of its own logon session instead of the service's (A13).
    let mut desktop = [0u16];
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.lpDesktop = desktop.as_mut_ptr();
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = input.raw();
    startup.StartupInfo.hStdOutput = output.raw();
    startup.StartupInfo.hStdError = null_device.raw();
    startup.lpAttributeList = attributes.ptr();
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessAsUserW(
            token.raw(),
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            environment.0,
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    let _thread = OwnedHandle::new(process.hThread)?;
    Ok((job, OwnedHandle::new(process.hProcess)?))
}

/// `NUL` for the relay's standard error, so no stray output of it enters
/// the framed protocol.
fn null_output() -> io::Result<OwnedHandle> {
    let name = wide("NUL");
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    OwnedHandle::new(handle)
}

fn inheritable(handle: HANDLE) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn bad(detail: &str) -> Refusal {
    (HelperErrorCode::BadRequest, detail.to_string())
}

fn failed(error: io::Error) -> Refusal {
    (HelperErrorCode::StartFailed, error.to_string())
}

fn outdated(error: &io::Error) -> Refusal {
    (HelperErrorCode::Outdated, error.to_string())
}
