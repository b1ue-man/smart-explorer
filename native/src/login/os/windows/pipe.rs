//! Non-network named pipes, explicit service SID, kernel token and PID checks.
use super::{
    security::{self, wide, Descriptor, Handle},
    PIPE,
};
use crate::login::os::shared::{denied, installation};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE,
    },
    Security::{self, TokenGroups, TokenRestrictedSids, SE_GROUP_ENABLED, TOKEN_QUERY},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    },
    System::{
        Pipes::{
            self, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        },
        Threading::{GetCurrentThread, OpenThreadToken},
    },
};

pub(crate) struct ClientStream {
    file: File,
}
impl ClientStream {
    pub(super) fn from_handle(handle: Handle) -> Self {
        let raw = handle.0;
        std::mem::forget(handle);
        Self {
            file: unsafe { File::from_raw_handle(raw.cast()) },
        }
    }
    pub(super) fn clone_stream(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
        })
    }
    pub(super) fn raw(&self) -> HANDLE {
        self.file.as_raw_handle().cast()
    }
}
impl Read for ClientStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match self.file.read(bytes) {
                Err(error) if matches!(error.raw_os_error(), Some(code) if code == ERROR_NO_DATA as i32 || code == ERROR_PIPE_LISTENING as i32) =>
                {
                    if Instant::now() >= end {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "login pipe read deadline",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => {
                    return Ok(0)
                }
                result => return result,
            }
        }
    }
}
impl Write for ClientStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match self.file.write(bytes) {
                Ok(0) if !bytes.is_empty() => {
                    if Instant::now() >= end {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "login pipe write deadline",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    } // No blocking FlushFileBuffers across a trust boundary.
}
pub(super) fn create(name: &str, gateway_sid: Option<&str>) -> io::Result<ClientStream> {
    let mut sddl = "O:SYG:SYD:P(A;;GA;;;SY)".to_string();
    if let Some(sid) = gateway_sid {
        sddl.push_str(&format!("(A;;0x00100003;;;{sid})"));
    }
    let descriptor = Descriptor::new(&sddl)?;
    let handle = unsafe {
        Pipes::CreateNamedPipeW(
            wide(name).as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            5000,
            &descriptor.attributes(),
        )
    };
    Ok(ClientStream::from_handle(Handle::new(handle)?))
}
pub(super) fn accept(
    pipe: &ClientStream,
    limit: Duration,
    stopping: impl Fn() -> bool,
) -> io::Result<()> {
    let end = Instant::now() + limit;
    loop {
        if stopping() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "login service stopping",
            ));
        }
        if unsafe { Pipes::ConnectNamedPipe(pipe.raw(), null_mut()) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == ERROR_PIPE_CONNECTED as i32 => return Ok(()),
            Some(code) if code == ERROR_PIPE_LISTENING as i32 || code == ERROR_NO_DATA as i32 => {
                if Instant::now() >= end {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "login pipe accept deadline",
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => return Err(error),
        }
    }
}
pub(super) fn client_pid(pipe: &ClientStream) -> io::Result<u32> {
    let mut pid = 0;
    if unsafe { Pipes::GetNamedPipeClientProcessId(pipe.raw(), &mut pid) } == 0 || pid == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}
fn group_enabled(
    token: HANDLE,
    class: Security::TOKEN_INFORMATION_CLASS,
    wanted: &str,
    restricted: bool,
) -> io::Result<bool> {
    let data = security::token_data(token, class)?;
    let groups = unsafe { &*data.as_ptr().cast::<Security::TOKEN_GROUPS>() };
    let count = groups.GroupCount as usize;
    if count > 4096 {
        return Err(denied("login token has too many groups"));
    }
    let items = unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), count) };
    for group in items {
        if (restricted || group.Attributes & SE_GROUP_ENABLED != 0)
            && security::sid_string(group.Sid)? == wanted
        {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn authorize_gateway(pipe: &ClientStream, sid: &str) -> io::Result<()> {
    if unsafe { Pipes::ImpersonateNamedPipeClient(pipe.raw()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            unsafe {
                Security::RevertToSelf();
            }
        }
    }
    let _revert = Revert;
    let mut raw = null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle::new(raw)?;
    if security::token_user(token.0)? != "S-1-5-19"
        || !group_enabled(token.0, TokenGroups, sid, false)?
        || !group_enabled(token.0, TokenRestrictedSids, sid, true)?
    {
        return Err(denied(
            "login IPC requires the exact restricted LocalService gateway token",
        ));
    }
    Ok(())
}
fn open(name: &str) -> io::Result<ClientStream> {
    let raw = unsafe {
        CreateFileW(
            wide(name).as_ptr(),
            0x00100003,
            0,
            null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            null_mut(),
        )
    };
    let stream = ClientStream::from_handle(Handle::new(raw)?);
    let mode = PIPE_NOWAIT | PIPE_READMODE_BYTE;
    if unsafe { Pipes::SetNamedPipeHandleState(stream.raw(), &mode, null(), null()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(stream)
}
fn verify_server(stream: &ClientStream, expected_pid: Option<u32>) -> io::Result<()> {
    let mut pid = 0;
    if unsafe { Pipes::GetNamedPipeServerProcessId(stream.raw(), &mut pid) } == 0
        || expected_pid.is_some_and(|expected| expected != pid)
    {
        return Err(denied(
            "login pipe server process does not match the protected broker",
        ));
    }
    super::session::verify_system_process(pid)?;
    Ok(())
}
pub(crate) fn require_gateway(policy: &crate::login::Policy) -> io::Result<()> {
    super::validate_gateway_identity(&policy.gateway_identity)?;
    let token = security::process_token(unsafe {
        windows_sys::Win32::System::Threading::GetCurrentProcess()
    })?;
    if security::token_user(token.0)? != "S-1-5-19"
        || !group_enabled(token.0, TokenGroups, &policy.gateway_identity, false)?
        || !group_enabled(token.0, TokenRestrictedSids, &policy.gateway_identity, true)?
    {
        return Err(denied(
            "only the restricted gateway service may use private machine state and login IPC",
        ));
    }
    Ok(())
}
pub(crate) fn connect() -> io::Result<ClientStream> {
    let policy = installation::policy()?;
    require_gateway(&policy)?;
    let stream = open(PIPE)?;
    verify_server(&stream, None)?;
    Ok(stream)
}
pub(crate) fn helper_channel() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    security::require_system()?;
    let nonce = std::env::var("SE_LOGIN_HELPER_CHANNEL")
        .map_err(|_| denied("login helper has no broker-created channel"))?;
    if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(denied("invalid login helper channel"));
    }
    let parent: u32 = std::env::var("SE_LOGIN_HELPER_PARENT")
        .map_err(|_| denied("missing login helper parent"))?
        .parse()
        .map_err(io::Error::other)?;
    let stream = open(&format!(r"\\.\pipe\SmartExplorerLoginHelper-{nonce}"))?;
    verify_server(&stream, Some(parent))?;
    Ok((Box::new(stream.clone_stream()?), Box::new(stream)))
}

/// Replay the byte read before token impersonation into the bounded frame parser.
pub(super) struct Prefixed<'a> {
    first: Option<u8>,
    pipe: &'a mut ClientStream,
}
impl<'a> Prefixed<'a> {
    pub(super) fn new(pipe: &'a mut ClientStream, first: u8) -> Self {
        Self {
            first: Some(first),
            pipe,
        }
    }
}
impl Read for Prefixed<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if let Some(first) = self.first.take() {
            bytes[0] = first;
            return Ok(1);
        }
        self.pipe.read(bytes)
    }
}
impl Write for Prefixed<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pipe.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.pipe.flush()
    }
}
