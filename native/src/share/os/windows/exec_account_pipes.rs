//! The worker's pipes to the Windows account service (PRM-M7, C2): the
//! connection to the service's pipe, with a time limit on every read and
//! write, and the two pipes of one job session that the relay uses as
//! standard input and output.
//!
//! The service's pipe is trusted only when SYSTEM owns it: a user could
//! create a pipe of that name while the service is stopped, but only SYSTEM
//! may give a pipe the owner SYSTEM. The worker allows the service to
//! identify it, not to act as it (`SECURITY_IDENTIFICATION`).
//!
//! The job pipes `\\.\pipe\SmartExplorerExec-<128-bit id>-in` (the worker
//! writes) and `…-out` (the worker reads) have one instance each that no one
//! could have created before (`FILE_FLAG_FIRST_PIPE_INSTANCE`), refuse
//! remote clients, may be opened only by SYSTEM, and belong to the worker's
//! user, which the service checks before it hands them to a relay.
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::SE_KERNEL_OBJECT;
use windows_sys::Win32::Security::WinLocalSystemSid;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_GENERIC_READ, FILE_WRITE_DATA, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, WaitNamedPipeW,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::share::exec_types::ExecId;

use super::exec_account_config::SERVICE_PIPE;
use super::exec_account_security::{current_user_sid, owner_of, wide, SecurityDescriptor, Sid};
use super::exec_handles::OwnedHandle;

/// Every read and write of the worker waits this long at most; a start
/// loads the account's profile first.
const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the worker waits for a free instance of the service's pipe.
const BUSY_WAIT: Duration = Duration::from_secs(10);
/// Access a registered user has to the service's pipe: read, and write
/// data. Never `FILE_CREATE_PIPE_INSTANCE`, so no user can add an instance.
pub(super) const CLIENT_ACCESS: u32 = FILE_GENERIC_READ | FILE_WRITE_DATA;
/// Buffer size of each direction of a job pipe.
const JOB_PIPE_BUFFER: u32 = 64 * 1024;

/// The worker's connection to the service's pipe.
pub(super) struct ControlPipe {
    handle: OwnedHandle,
    event: OwnedHandle,
}

impl ControlPipe {
    /// Opens the service's pipe; `Ok(None)` when it does not exist, that is
    /// when the service does not run.
    pub(super) fn open() -> io::Result<Option<Self>> {
        let name = wide(SERVICE_PIPE);
        let deadline = Instant::now() + BUSY_WAIT;
        loop {
            let flags = FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    CLIENT_ACCESS,
                    0,
                    null(),
                    OPEN_EXISTING,
                    flags,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                return Self::checked(OwnedHandle::new(handle)?).map(Some);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Some(ERROR_PIPE_BUSY) if Instant::now() < deadline => {
                    unsafe { WaitNamedPipeW(name.as_ptr(), 1_000) };
                }
                _ => return Err(error),
            }
        }
    }

    fn checked(handle: OwnedHandle) -> io::Result<Self> {
        let owner = owner_of(handle.raw(), SE_KERNEL_OBJECT)?;
        if owner != Sid::well_known(WinLocalSystemSid)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{SERVICE_PIPE} does not belong to SYSTEM, so the account service does not serve it"),
            ));
        }
        let event = OwnedHandle::new(unsafe { CreateEventW(null(), 1, 0, null()) })?;
        Ok(Self { handle, event })
    }

    /// One overlapped read or write that may take `IO_TIMEOUT`.
    fn transfer(&self, buffer: *mut u8, length: u32, write: bool) -> io::Result<usize> {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.event.raw();
        let started = unsafe {
            if write {
                WriteFile(
                    self.handle.raw(),
                    buffer,
                    length,
                    null_mut(),
                    &mut overlapped,
                )
            } else {
                ReadFile(
                    self.handle.raw(),
                    buffer,
                    length,
                    null_mut(),
                    &mut overlapped,
                )
            }
        };
        if started == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return ended(error, write);
            }
            let waited = unsafe { WaitForSingleObject(self.event.raw(), timeout_ms()) };
            if waited != WAIT_OBJECT_0 {
                let mut ignored = 0u32;
                // The operation must end before `overlapped` goes away.
                unsafe {
                    CancelIoEx(self.handle.raw(), &overlapped);
                    GetOverlappedResult(self.handle.raw(), &overlapped, &mut ignored, 1);
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the account service did not answer in time",
                ));
            }
        }
        let mut done = 0u32;
        if unsafe { GetOverlappedResult(self.handle.raw(), &overlapped, &mut done, 0) } == 0 {
            return ended(io::Error::last_os_error(), write);
        }
        Ok(done as usize)
    }
}

impl Read for ControlPipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(u32::MAX as usize) as u32;
        self.transfer(buffer.as_mut_ptr(), length, false)
    }
}

impl Write for ControlPipe {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = buffer.len().min(u32::MAX as usize) as u32;
        self.transfer(buffer.as_ptr().cast_mut(), length, true)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A closed pipe ends a read like an end of file.
fn ended(error: io::Error, write: bool) -> io::Result<usize> {
    if !write && error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
        Ok(0)
    } else {
        Err(error)
    }
}

fn timeout_ms() -> u32 {
    IO_TIMEOUT.as_millis().min(u128::from(u32::MAX - 1)) as u32
}

/// The name of the job pipe `end` (`in` or `out`) of the session `id`.
pub(super) fn job_pipe_name(id: &str, end: &str) -> String {
    format!(r"\\.\pipe\SmartExplorerExec-{id}-{end}")
}

/// The session id of `name` when it is exactly the job pipe `end` of an id
/// of 32 lowercase hex digits; the service opens no other pipe.
pub(super) fn job_pipe_id<'a>(name: &'a str, end: &str) -> Option<&'a str> {
    let id = name
        .strip_prefix(r"\\.\pipe\SmartExplorerExec-")?
        .strip_suffix(end)?
        .strip_suffix('-')?;
    let hex = id
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (id.len() == 32 && hex).then_some(id)
}

/// The two pipes of one job session.
pub(super) struct JobPipes {
    id: String,
    input: OwnedHandle,
    output: OwnedHandle,
}

impl JobPipes {
    pub(super) fn create() -> io::Result<Self> {
        let id = ExecId::generate()?.as_str().to_string();
        let owner = current_user_sid()?.text()?;
        let descriptor = SecurityDescriptor::from_sddl(&format!("O:{owner}D:P(A;;GA;;;SY)"))?;
        let input = create_job_pipe(&job_pipe_name(&id, "in"), PIPE_ACCESS_OUTBOUND, &descriptor)?;
        let output = create_job_pipe(&job_pipe_name(&id, "out"), PIPE_ACCESS_INBOUND, &descriptor)?;
        Ok(Self { id, input, output })
    }

    /// `(pipe_in, pipe_out)` of the start request (D.2.7).
    pub(super) fn names(&self) -> (String, String) {
        (
            job_pipe_name(&self.id, "in"),
            job_pipe_name(&self.id, "out"),
        )
    }

    /// The worker's ends once the service opened the relay's ends: the
    /// writer of the relay's input and the reader of its output.
    pub(super) fn connect(self) -> io::Result<(File, File)> {
        for pipe in [self.input.raw(), self.output.raw()] {
            connected(pipe)?;
        }
        // SAFETY: both handles are open and owned by nothing else from here on.
        let writer = unsafe { File::from_raw_handle(self.input.into_raw().cast()) };
        let reader = unsafe { File::from_raw_handle(self.output.into_raw().cast()) };
        Ok((writer, reader))
    }
}

fn create_job_pipe(
    name: &str,
    direction: u32,
    descriptor: &SecurityDescriptor,
) -> io::Result<OwnedHandle> {
    let attributes = descriptor.attributes(false);
    let name = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            direction | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            JOB_PIPE_BUFFER,
            JOB_PIPE_BUFFER,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    OwnedHandle::new(handle)
}

/// The service opened the client end of `pipe` (else waiting for it could
/// block forever); a relay that already ended counts as an error.
fn connected(pipe: HANDLE) -> io::Result<()> {
    let mut client = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe, &mut client) } == 0 || client == 0 {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "the account service did not open the pipes of the job",
        ));
    }
    if unsafe { ConnectNamedPipe(pipe, null_mut()) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
            return Err(error);
        }
    }
    Ok(())
}
