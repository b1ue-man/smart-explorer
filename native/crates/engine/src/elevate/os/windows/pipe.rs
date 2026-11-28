//! The two byte pipes between se and its elevated helper. The caller creates
//! `NAME-in` (it writes, the helper reads) and `NAME-out` (the helper
//! writes) with one instance each (`FILE_FLAG_FIRST_PIPE_INSTANCE`: a pipe
//! of that name created by anyone else first makes the creation fail),
//! `PIPE_REJECT_REMOTE_CLIENTS` and the ACL `D:P(D;;GA;;;NU)(A;;GA;;;<user>)
//! (A;;GA;;;BA)`: the default ACL would let Everyone read (MS-pipe); NETWORK
//! is denied; the Administrators are allowed, because an elevation with
//! another administrator's credentials runs the helper as that account.
//! The helper opens both with `SECURITY_IDENTIFICATION`, so the
//! unelevated end cannot impersonate it.
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    PIPE_ACCESS_OUTBOUND, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const BUFFER: u32 = 64 * 1024;
/// How long the caller waits for the started helper to connect.
const CONNECT_WAIT: Duration = Duration::from_secs(60);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The security descriptor of the ACL above.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: LocalAlloc'ed by ConvertStringSecurityDescriptor…, freed once.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn descriptor() -> io::Result<Descriptor> {
    let sddl = format!(
        "D:P(D;;GA;;;NU)(A;;GA;;;{})(A;;GA;;;BA)",
        super::token::user_sid_text()?
    );
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: a NUL-terminated SDDL string and an out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(&sddl).as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Descriptor(descriptor))
}

/// The caller's ends, created and not yet connected.
pub(super) struct ServerPipes {
    pub(super) to_helper: File,
    pub(super) from_helper: File,
    name: String,
}

pub(super) fn create(name: &str) -> io::Result<ServerPipes> {
    let descriptor = descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let make = |suffix: &str, access: u32| -> io::Result<File> {
        // SAFETY: NUL-terminated name, attributes that live for the call.
        let handle = unsafe {
            CreateNamedPipeW(
                wide(&format!("{name}{suffix}")).as_ptr(),
                access | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER,
                BUFFER,
                0,
                &attributes,
            )
        };
        owned(handle)
    };
    Ok(ServerPipes {
        to_helper: make("-in", PIPE_ACCESS_OUTBOUND)?,
        from_helper: make("-out", PIPE_ACCESS_INBOUND)?,
        name: name.to_string(),
    })
}

fn owned(handle: HANDLE) -> io::Result<File> {
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle this function now owns.
    Ok(unsafe { File::from_raw_handle(handle) })
}

impl ServerPipes {
    /// Waits until the helper `process` (its handle and id) connected both
    /// pipes. A helper that ends first, or another process that connected,
    /// fails the wait.
    pub(super) fn accept(&self, process: HANDLE, pid: u32) -> io::Result<()> {
        for (pipe, suffix, access) in [
            (&self.to_helper, "-in", GENERIC_READ),
            (&self.from_helper, "-out", GENERIC_WRITE),
        ] {
            connect(pipe, &format!("{}{suffix}", self.name), access, process)?;
            let mut client = 0u32;
            // SAFETY: a connected server end and an out pointer.
            if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut client) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if client != pid {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "another process connected to the elevation pipe",
                ));
            }
        }
        Ok(())
    }
}

/// `ConnectNamedPipe` on its own thread; when the helper ends or the wait
/// runs out first, this process connects itself to end the call (its pid
/// then fails the check above).
fn connect(pipe: &File, name: &str, access: u32, process: HANDLE) -> io::Result<()> {
    let raw = pipe.as_raw_handle() as usize;
    let (done, finished) = mpsc::channel();
    std::thread::Builder::new()
        .name("se-elevate-accept".into())
        .spawn(move || {
            // SAFETY: the pipe outlives this call (the caller waits for it).
            let ok = unsafe { ConnectNamedPipe(raw as HANDLE, null_mut()) } != 0;
            let error = io::Error::last_os_error();
            let connected = ok || error.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32);
            let _ = done.send(if connected { Ok(()) } else { Err(error) });
        })?;
    let started = std::time::Instant::now();
    loop {
        match finished.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("the pipe wait ended without a result"))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // SAFETY: a process handle the caller holds.
        let alive = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
        if !alive || started.elapsed() > CONNECT_WAIT {
            let _unblock = open_client(name, access);
            let _ = finished.recv_timeout(Duration::from_secs(5));
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the elevated helper did not connect",
            ));
        }
    }
}

fn open_client(name: &str, access: u32) -> io::Result<File> {
    // SAFETY: NUL-terminated name; the handle is owned by the File.
    let handle = unsafe {
        CreateFileW(
            wide(name).as_ptr(),
            access,
            0,
            null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            null_mut(),
        )
    };
    owned(handle)
}

/// The helper's ends: reads `NAME-in`, writes `NAME-out`; both must belong
/// to the process `server_pid`.
pub(super) fn open_helper_ends(name: &str, server_pid: u32) -> io::Result<(File, File)> {
    let from_caller = open_client(&format!("{name}-in"), GENERIC_READ)?;
    let to_caller = open_client(&format!("{name}-out"), GENERIC_WRITE)?;
    for pipe in [&from_caller, &to_caller] {
        let mut server = 0u32;
        // SAFETY: a connected client end and an out pointer.
        if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if server != server_pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the elevation pipe belongs to another process",
            ));
        }
    }
    Ok((from_caller, to_caller))
}
