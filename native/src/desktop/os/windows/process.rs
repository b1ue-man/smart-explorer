//! Processes of se Desktop on Windows (DSK-M3, C5.5, C5.7): the shell starts
//! detached from its caller, a surface starts with pipes for its channel, and
//! a process is ended by PID only after `QueryFullProcessImageNameW` named
//! the expected program on the same handle (no PID-reuse race).
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

/// Exit code given to a process ended by force.
const KILLED: u32 = 1;

/// Starts `exe args` detached from the caller: no console, its own process
/// group, null standard streams. The caller's standard handles are made
/// non-inheritable first, so an automation host that pipes `se` never waits
/// for the shell (same reason as the daemon start in `autostart`).
pub(crate) fn spawn_detached(exe: &Path, args: &[&str]) -> io::Result<Child> {
    clear_standard_handle_inheritance()?;
    Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
}

/// Starts a surface with stdin/stdout as its channel to the shell.
pub(crate) fn spawn_piped(exe: &Path, args: &[String]) -> io::Result<Child> {
    Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(pid: u32, access: u32) -> Option<ProcessHandle> {
        if pid == 0 {
            return None;
        }
        let handle = unsafe { OpenProcess(access, 0, pid) };
        (!handle.is_null()).then_some(ProcessHandle(handle))
    }

    fn image(&self) -> Option<PathBuf> {
        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size)
        };
        if ok == 0 {
            return None;
        }
        buffer.truncate(size as usize);
        Some(PathBuf::from(OsString::from_wide(&buffer)))
    }

    fn running(&self) -> bool {
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(self.0, &mut code) };
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn normalized(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_lowercase()
}

/// Whether the image `found` is the program `expected` (case-insensitive,
/// with or without the `\\?\` prefix of a canonical path).
pub(crate) fn same_image(found: &Path, expected: &Path) -> bool {
    let found = normalized(found);
    if found == normalized(expected) {
        return true;
    }
    std::fs::canonicalize(expected).is_ok_and(|canonical| found == normalized(&canonical))
}

/// The process `pid` runs and its image is `expected`.
pub(crate) fn alive_with_image(pid: u32, expected: &Path) -> bool {
    ProcessHandle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION).is_some_and(|process| {
        process.running()
            && process
                .image()
                .is_some_and(|image| same_image(&image, expected))
    })
}

/// Ends `pid` only when its image is `expected`; `Ok(false)` when the process
/// is gone or runs another program.
pub(crate) fn terminate_checked(pid: u32, expected: &Path) -> io::Result<bool> {
    let Some(process) =
        ProcessHandle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE)
    else {
        return Ok(false);
    };
    let matches = process.running()
        && process
            .image()
            .is_some_and(|image| same_image(&image, expected));
    if !matches {
        return Ok(false);
    }
    if unsafe { TerminateProcess(process.0, KILLED) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(true)
}

fn clear_standard_handle_inheritance() -> io::Result<()> {
    for handle in [
        io::stdin().as_raw_handle(),
        io::stdout().as_raw_handle(),
        io::stderr().as_raw_handle(),
    ] {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
