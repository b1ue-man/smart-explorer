//! Local, message-framed IPC with bounded memory and peer-aware deadlines.
use crate::local_access::protocol::MAX_FRAME;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, GENERIC_READ, GENERIC_WRITE,
        HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    },
    Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    },
    System::{Pipes::*, Threading::WaitForSingleObject},
};

pub(super) struct Pipe(File);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

impl Pipe {
    pub(super) fn server(name: &str) -> io::Result<Self> {
        let handle = unsafe {
            CreateNamedPipeW(
                wide(name).as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_FRAME as u32,
                MAX_FRAME as u32,
                0,
                null(),
            )
        };
        Self::from_created(handle)
    }

    pub(super) fn client(name: &str) -> io::Result<Self> {
        let handle = unsafe {
            CreateFileW(
                wide(name).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        };
        let pipe = Self::from_created(handle)?;
        let mode = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
        if unsafe { SetNamedPipeHandleState(pipe.raw(), &mode, null(), null()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pipe)
    }

    fn from_created(handle: HANDLE) -> io::Result<Self> {
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // This constructor takes ownership of a newly created OS handle.
        Ok(Self(unsafe { File::from_raw_handle(handle) }))
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    pub(super) fn accept(&self, peer: HANDLE) -> io::Result<()> {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if unsafe { ConnectNamedPipe(self.raw(), null_mut()) } != 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_PIPE_CONNECTED) => return Ok(()),
                Some(ERROR_PIPE_LISTENING) => pause(peer, end)?,
                _ => return Err(error),
            }
        }
    }

    pub(super) fn peer_id(&self, server: bool) -> io::Result<u32> {
        let mut pid = 0;
        let ok = unsafe {
            if server {
                GetNamedPipeClientProcessId(self.raw(), &mut pid)
            } else {
                GetNamedPipeServerProcessId(self.raw(), &mut pid)
            }
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    pub(super) fn send(&self, value: &impl Serialize, peer: HANDLE) -> io::Result<()> {
        let data = serde_json::to_vec(value).map_err(io::Error::other)?;
        if data.len() > MAX_FRAME {
            return Err(io::Error::other("Lesehelfer-Nachricht zu groß"));
        }
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let mut written = 0;
            if unsafe {
                WriteFile(
                    self.raw(),
                    data.as_ptr(),
                    data.len() as u32,
                    &mut written,
                    null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if written as usize == data.len() {
                return Ok(());
            }
            if written != 0 {
                return Err(io::Error::other("Unvollständige Lesehelfer-Nachricht"));
            }
            pause(peer, end)?;
        }
    }

    pub(super) fn receive<T: DeserializeOwned>(
        &self,
        peer: HANDLE,
        timeout: Duration,
    ) -> io::Result<T> {
        let mut data = vec![0; MAX_FRAME];
        let end = Instant::now() + timeout;
        loop {
            let mut count = 0;
            if unsafe {
                ReadFile(
                    self.raw(),
                    data.as_mut_ptr(),
                    data.len() as u32,
                    &mut count,
                    null_mut(),
                )
            } != 0
            {
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                return serde_json::from_slice(&data[..count as usize])
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_DATA as i32) {
                return Err(error);
            }
            pause(peer, end)?;
        }
    }
}

fn pause(peer: HANDLE, deadline: Instant) -> io::Result<()> {
    if unsafe { WaitForSingleObject(peer, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::from(io::ErrorKind::BrokenPipe));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
}
