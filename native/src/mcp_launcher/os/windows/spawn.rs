//! Exact application path plus an explicit inheritable-handle allowlist.
use crate::mcp_launcher::GenerationChild;
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

pub(in crate::mcp_launcher) struct NativeChild {
    handle: OwnedHandle,
    pid: u32,
}
impl NativeChild {
    pub(in crate::mcp_launcher) fn id(&self) -> u32 {
        self.pid
    }
    pub(in crate::mcp_launcher) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.handle.as_raw_handle() as _, &mut code) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok((code != STILL_ACTIVE as u32).then(|| ExitStatus::from_raw(code)))
    }
    pub(in crate::mcp_launcher) fn wait(&mut self) -> std::io::Result<ExitStatus> {
        unsafe {
            WaitForSingleObject(self.handle.as_raw_handle() as _, INFINITE);
        }
        self.try_wait()?
            .ok_or_else(|| std::io::Error::other("runtime_wait_failed"))
    }
    pub(in crate::mcp_launcher) fn kill(&mut self) -> std::io::Result<()> {
        if unsafe { TerminateProcess(self.handle.as_raw_handle() as _, 1) } == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
pub(in crate::mcp_launcher) fn request_stop(_pid: u32) -> bool {
    false
}
fn pipe() -> Result<(File, File), String> {
    let mut read = std::ptr::null_mut();
    let mut write = read;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe {
        (
            File::from_raw_handle(read as _),
            File::from_raw_handle(write as _),
        )
    })
}
fn inheritable(source: HANDLE) -> Result<File, String> {
    let process = unsafe { GetCurrentProcess() };
    let mut copy = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            source,
            process,
            &mut copy,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_handle(copy as _) })
}
fn quoted(value: &std::ffi::OsStr) -> Vec<u16> {
    let mut out = vec![b'"' as u16];
    let mut slashes = 0;
    for c in value.encode_wide() {
        if c == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if c == b'"' as u16 {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    out.push(b'"' as u16);
    out
}
pub(in crate::mcp_launcher) fn spawn_verified(
    command: Command,
    image: File,
    lifecycle: bool,
    capture: bool,
    input: bool,
) -> Result<GenerationChild, String> {
    spawn_impl(command, image, lifecycle, capture, input, true)
}
fn spawn_impl(
    mut command: Command,
    image: File,
    lifecycle: bool,
    capture: bool,
    input: bool,
    wait_ack: bool,
) -> Result<GenerationChild, String> {
    let (mut ack_read, ack_write) = pipe()?;
    let (life_read, life_write) = pipe()?;
    if wait_ack {
        command.env(
            "SE_MCP_ADMISSION_CHANNEL",
            (ack_write.as_raw_handle() as usize).to_string(),
        );
        command.env(
            "SE_MCP_LIFECYCLE_HANDLE",
            (life_read.as_raw_handle() as usize).to_string(),
        );
        command.env("SE_MCP_LIFECYCLE", if lifecycle { "1" } else { "0" });
    } else {
        command
            .env_remove("SE_MCP_ADMISSION_CHANNEL")
            .env_remove("SE_MCP_LIFECYCLE")
            .env_remove("SE_MCP_LIFECYCLE_HANDLE")
            .env_remove("SE_MCP_MAINTENANCE");
    }
    let (stdin_child, stdin_parent) = if capture {
        if input {
            let (r, w) = pipe()?;
            (r, Some(w))
        } else {
            (
                std::fs::OpenOptions::new()
                    .read(true)
                    .open("NUL")
                    .map_err(|e| e.to_string())?,
                None,
            )
        }
    } else {
        (
            inheritable(unsafe { GetStdHandle(STD_INPUT_HANDLE) })?,
            None,
        )
    };
    let (stdout_parent, stdout_child) = if capture {
        let (r, w) = pipe()?;
        (Some(r), w)
    } else {
        (
            None,
            inheritable(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) })?,
        )
    };
    let (stderr_parent, stderr_child) = if capture {
        let (r, w) = pipe()?;
        (Some(r), w)
    } else {
        (
            None,
            inheritable(unsafe { GetStdHandle(STD_ERROR_HANDLE) })?,
        )
    };
    let mut handles = [
        stdin_child.as_raw_handle() as HANDLE,
        stdout_child.as_raw_handle() as HANDLE,
        stderr_child.as_raw_handle() as HANDLE,
        ack_write.as_raw_handle() as HANDLE,
        life_read.as_raw_handle() as HANDLE,
    ];
    for handle in handles {
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    let mut size = 0;
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
    }
    let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
    let list = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    struct Attr(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for Attr {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.0);
            }
        }
    }
    let _attributes = Attr(list);
    if unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_mut_ptr() as _,
            std::mem::size_of_val(&handles),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let application = command
        .get_program()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut line = quoted(command.get_program());
    for arg in command.get_args() {
        line.push(b' ' as u16);
        line.extend(quoted(arg));
    }
    line.push(0);
    let env = super::environment::block(&command)?;
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of_val(&startup) as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = list;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
            env.as_ptr() as _,
            std::ptr::null(),
            &startup.StartupInfo,
            &mut info,
        )
    } == 0
    {
        return Err(format!(
            "runtime_CreateProcessW_failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        CloseHandle(info.hThread);
    }
    let mut process = NativeChild {
        handle: unsafe { OwnedHandle::from_raw_handle(info.hProcess as _) },
        pid: info.dwProcessId,
    };
    drop(image);
    drop(stdin_child);
    drop(stdout_child);
    drop(stderr_child);
    drop(ack_write);
    drop(life_read);
    if wait_ack {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut ready = [0];
        loop {
            let mut available = 0;
            if unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    ack_read.as_raw_handle() as _,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                let _ = process.kill();
                return Err("runtime_admission_channel_closed".into());
            }
            if available > 0 {
                ack_read.read_exact(&mut ready).map_err(|e| e.to_string())?;
                break;
            }
            if std::time::Instant::now() >= deadline
                || process.try_wait().map_err(|e| e.to_string())?.is_some()
            {
                let _ = process.kill();
                let _ = process.wait();
                return Err("runtime_admission_not_acknowledged".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if ready != [1] {
            let _ = process.kill();
            let _ = process.wait();
            return Err("runtime_admission_rejected".into());
        }
    }
    if command
        .get_envs()
        .any(|(key, value)| key == "SE_MCP_MAINTENANCE" && value == Some(std::ffi::OsStr::new("1")))
    {
        let mut release = &life_write;
        release.write_all(&[2]).map_err(|e| e.to_string())?;
    }
    Ok(GenerationChild {
        stdin: stdin_parent.map(|p| Box::new(p) as Box<dyn Write + Send>),
        stdout: stdout_parent.map(|p| Box::new(p) as Box<dyn Read + Send>),
        stderr: stderr_parent.map(|p| Box::new(p) as Box<dyn Read + Send>),
        process,
        _lifecycle: if lifecycle { Some(life_write) } else { None },
    })
}
pub(in crate::mcp_launcher) fn take_admission_channel(
) -> Result<Option<(File, bool, Option<File>)>, String> {
    let Some(raw) = std::env::var_os("SE_MCP_ADMISSION_CHANNEL") else {
        return Ok(None);
    };
    let handle = raw
        .to_str()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|h| *h > 0)
        .ok_or("runtime_admission_channel_invalid")?;
    std::env::remove_var("SE_MCP_ADMISSION_CHANNEL");
    let mut ack = unsafe { File::from_raw_handle(handle as _) };
    if unsafe { SetHandleInformation(ack.as_raw_handle() as _, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let lifecycle =
        std::env::var_os("SE_MCP_LIFECYCLE").as_deref() == Some(std::ffi::OsStr::new("1"));
    std::env::remove_var("SE_MCP_LIFECYCLE");
    let read = std::env::var("SE_MCP_LIFECYCLE_HANDLE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|h| *h > 0)
        .ok_or("runtime_lifecycle_handle_invalid")?;
    std::env::remove_var("SE_MCP_LIFECYCLE_HANDLE");
    let life = unsafe { File::from_raw_handle(read as _) };
    if unsafe { SetHandleInformation(life.as_raw_handle() as _, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(Some((life, lifecycle, Some(ack))))
}
pub(in crate::mcp_launcher) fn acknowledge(channel: &mut File) -> Result<(), String> {
    channel.write_all(&[1]).map_err(|e| e.to_string())
}
pub(in crate::mcp_launcher) fn spawn_legacy(
    mut command: Command,
    input: bool,
) -> Result<GenerationChild, String> {
    use std::process::Stdio;
    command
        .stdin(if input { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut duplicate = std::ptr::null_mut();
    let own = unsafe { GetCurrentProcess() };
    if unsafe {
        DuplicateHandle(
            own,
            child.as_raw_handle() as _,
            own,
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(std::io::Error::last_os_error().to_string());
    }
    let process = NativeChild {
        handle: unsafe { OwnedHandle::from_raw_handle(duplicate as _) },
        pid: child.id(),
    };
    Ok(GenerationChild {
        stdin: child
            .stdin
            .take()
            .map(|p| Box::new(p) as Box<dyn Write + Send>),
        stdout: child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        stderr: child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        process,
        _lifecycle: None,
    })
}

pub(in crate::mcp_launcher) fn run_stable_launcher(
    command: Command,
    image: File,
) -> Result<i32, String> {
    let mut child = spawn_impl(command, image, false, false, false, false)?;
    child
        .wait()
        .map(|s| s.code().unwrap_or(1))
        .map_err(|e| e.to_string())
}

pub(in crate::mcp_launcher) fn spawn_current_daemon(
    command: Command,
    image: File,
) -> Result<(), String> {
    let _child = spawn_impl(command, image, false, true, false, false)?;
    Ok(())
}

pub(in crate::mcp_launcher) fn spawn_installation_probe(
    command: Command,
    image: File,
) -> Result<GenerationChild, String> {
    spawn_impl(command, image, false, true, false, false)
}
