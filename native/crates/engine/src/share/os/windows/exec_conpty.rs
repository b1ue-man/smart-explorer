//! ConPTY (Windows pseudoconsole) host for terminal Exec starts.
//!
//! `CreatePseudoConsole`, `ResizePseudoConsole` and `ClosePseudoConsole` are
//! resolved from kernel32.dll once at run time and are never linked at load
//! time: a load-time import would keep `se.exe` and the app from starting on
//! a Windows older than 10 1809, which only lacks the terminal capability
//! (plan E12). The caller owns the I/O threads: one thread per pipe direction,
//! and `close_console` never runs on the thread that reads the output.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::{size_of, transmute, zeroed};
use std::os::windows::io::FromRawHandle;
use std::sync::OnceLock;
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::{
    FALSE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, COORD, HPCON};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::exec_handles::{AttributeList, OwnedHandle};
use crate::share::ExecTerminalSize;

#[cfg(all(test, windows))]
#[path = "exec_conpty_tests.rs"]
mod tests;

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HPCON);

#[derive(Clone, Copy)]
struct Api {
    create: CreateFn,
    resize: ResizeFn,
    close: CloseFn,
}

/// The three pseudoconsole functions, or `None` on a Windows without them.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load_api).as_ref()
}

fn load_api() -> Option<Api> {
    let module_name = wide("kernel32.dll");
    let module = unsafe { GetModuleHandleW(module_name.as_ptr()) };
    if module.is_null() {
        return None;
    }
    let create = unsafe { GetProcAddress(module, b"CreatePseudoConsole\0".as_ptr()) }?;
    let resize = unsafe { GetProcAddress(module, b"ResizePseudoConsole\0".as_ptr()) }?;
    let close = unsafe { GetProcAddress(module, b"ClosePseudoConsole\0".as_ptr()) }?;
    // SAFETY: the three exports have exactly these signatures (Microsoft Learn:
    // CreatePseudoConsole, ResizePseudoConsole, ClosePseudoConsole).
    Some(unsafe {
        Api {
            create: transmute::<unsafe extern "system" fn() -> isize, CreateFn>(create),
            resize: transmute::<unsafe extern "system" fn() -> isize, ResizeFn>(resize),
            close: transmute::<unsafe extern "system" fn() -> isize, CloseFn>(close),
        }
    })
}

/// True when all three pseudoconsole functions exist in kernel32.dll.
pub(super) fn terminal_supported() -> bool {
    api().is_some()
}

/// A program and its command line, as the UTF-16 strings `CreateProcessW` takes.
pub(super) struct Program {
    /// Absolute path, NUL-terminated (`lpApplicationName`).
    application: Vec<u16>,
    /// NUL-terminated (`lpCommandLine`).
    command_line: Vec<u16>,
}

impl Program {
    /// A program of the Windows system directory, started by its absolute
    /// path. There is no search path, so nothing is taken from the working
    /// directory.
    pub(super) fn system(relative: &str, command_line: &str) -> io::Result<Self> {
        let mut application = system_directory()?;
        application.push(u16::from(b'\\'));
        application.extend(relative.encode_utf16());
        application.push(0);
        Ok(Self {
            application,
            command_line: wide(command_line),
        })
    }

    /// Windows PowerShell, interactive, with the user's profile.
    pub(super) fn login_shell() -> io::Result<Self> {
        Self::system(
            r"WindowsPowerShell\v1.0\powershell.exe",
            "powershell.exe -NoLogo",
        )
    }
}

/// One running process inside a pseudoconsole. The returned output reader and
/// input writer belong to the caller's threads.
pub(super) struct Conpty {
    console: Pseudoconsole,
    process: OwnedHandle,
    pid: u32,
}

/// Starts `program` in a new pseudoconsole of `size`. `environment` is the
/// complete environment of the process; `cwd` is its working directory (the
/// caller's own when `None`). Returns the process, the pseudoconsole output
/// (read until the pipe breaks) and the pseudoconsole input (keyboard bytes).
pub(super) fn start(
    program: &Program,
    environment: &BTreeMap<String, String>,
    cwd: Option<&str>,
    size: ExecTerminalSize,
) -> io::Result<(Conpty, File, File)> {
    let api = api().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "this Windows has no pseudoconsole (ConPTY)",
        )
    })?;
    // Ctrl+C typed into the pseudoconsole reaches its processes as a
    // CTRL_C_EVENT unless they ignore it, and that attribute is inherited
    // (SetConsoleCtrlHandler, Microsoft Learn), for example from an ancestor
    // started with CREATE_NEW_PROCESS_GROUP. Clearing it in this process
    // before the child is created gives the shell Ctrl+C as over SSH.
    // SAFETY: without a handler routine the call only changes that attribute.
    if unsafe { SetConsoleCtrlHandler(None, FALSE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // Pipes without security attributes are not inheritable; the child gets
    // its console from the pseudoconsole only.
    let (input_read, input_write) = plain_pipe()?;
    let (output_read, output_write) = plain_pipe()?;
    let console = Pseudoconsole::create(api, size, input_read.raw(), output_write.raw())?;

    let block = environment_block(environment);
    let directory = cwd.map(wide);
    let mut command_line = program.command_line.clone();
    let mut attributes = AttributeList::new(1)?;
    // The value is the HPCON itself, not a pointer to it (Microsoft sample).
    attributes.set(
        PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
        console.handle as *const c_void,
        size_of::<HPCON>(),
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // Without explicit invalid standard handles the child could inherit the
    // redirected handles of this supervisor (portable-pty does the same).
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.ptr();
    let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessW(
            program.application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            block.as_ptr().cast(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    let process = OwnedHandle::new(information.hProcess)?;
    let _thread = OwnedHandle::new(information.hThread)?;
    // The pseudoconsole holds its own copies of these two ends. Closing ours
    // lets the pipes break once the pseudoconsole closes its copies.
    drop(input_read);
    drop(output_write);
    let conpty = Conpty {
        console,
        process,
        pid: information.dwProcessId,
    };
    Ok((conpty, into_file(output_read), into_file(input_write)))
}

impl Conpty {
    pub(super) fn process_id(&self) -> u32 {
        self.pid
    }

    /// Sets the window size of the pseudoconsole.
    pub(super) fn resize(&self, size: ExecTerminalSize) -> io::Result<()> {
        self.console.resize(size)
    }

    /// Waits up to `timeout` for the root process; its exit code once it ended.
    pub(super) fn wait(&self, timeout: Duration) -> io::Result<Option<u32>> {
        // INFINITE is u32::MAX; a longer wait is capped just below it.
        let milliseconds = u32::try_from(timeout.as_millis())
            .unwrap_or(u32::MAX)
            .min(u32::MAX - 1);
        match unsafe { WaitForSingleObject(self.process.raw(), milliseconds) } {
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Some(code))
                }
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Ends the root process. A process that already ended is not an error.
    pub(super) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.process.raw(), 1) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match self.wait(Duration::ZERO) {
            Ok(Some(_)) => Ok(()),
            _ => Err(error),
        }
    }

    /// Closes the pseudoconsole on its own thread. Before Windows 11 24H2
    /// `ClosePseudoConsole` waits until the output pipe is drained, so the
    /// caller keeps reading the output until the pipe breaks. The returned
    /// thread ends when the call returns; `None` when the console was already
    /// closed or the thread could not be started (the console then stays open
    /// until the process ends, which the host's Job Object bounds).
    pub(super) fn close_console(&mut self) -> Option<JoinHandle<()>> {
        self.console.close_in_background()
    }
}

struct Pseudoconsole {
    api: &'static Api,
    /// 0 once closed.
    handle: HPCON,
}

impl Pseudoconsole {
    fn create(
        api: &'static Api,
        size: ExecTerminalSize,
        input: HANDLE,
        output: HANDLE,
    ) -> io::Result<Self> {
        let mut handle: HPCON = 0;
        let result = unsafe { (api.create)(to_coord(size)?, input, output, 0, &mut handle) };
        check(result)?;
        Ok(Self { api, handle })
    }

    fn resize(&self, size: ExecTerminalSize) -> io::Result<()> {
        if self.handle == 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the pseudoconsole is closed",
            ));
        }
        check(unsafe { (self.api.resize)(self.handle, to_coord(size)?) })
    }

    fn close_in_background(&mut self) -> Option<JoinHandle<()>> {
        let handle = std::mem::take(&mut self.handle);
        if handle == 0 {
            return None;
        }
        let close = self.api.close;
        std::thread::Builder::new()
            .name("exec-conpty-close".into())
            .spawn(move || unsafe { close(handle) })
            .ok()
    }
}

impl Drop for Pseudoconsole {
    fn drop(&mut self) {
        drop(self.close_in_background());
    }
}

fn to_coord(size: ExecTerminalSize) -> io::Result<COORD> {
    size.validate()?;
    Ok(COORD {
        X: i16::try_from(size.cols).map_err(|_| too_large())?,
        Y: i16::try_from(size.rows).map_err(|_| too_large())?,
    })
}

fn too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "terminal columns and rows must be between 1 and 32767",
    )
}

fn check(result: HRESULT) -> io::Result<()> {
    if result >= 0 {
        return Ok(());
    }
    let code = result as u32;
    // HRESULT_FROM_WIN32 values carry the Win32 error in the low 16 bits.
    if code >> 16 == 0x8007 {
        Err(io::Error::from_raw_os_error((code & 0xFFFF) as i32))
    } else {
        Err(io::Error::other(format!(
            "pseudoconsole call failed with HRESULT 0x{code:08X}"
        )))
    }
}

fn plain_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let read = OwnedHandle::new(read)?;
    let write = OwnedHandle::new(write)?;
    Ok((read, write))
}

fn into_file(handle: OwnedHandle) -> File {
    unsafe { File::from_raw_handle(handle.into_raw().cast()) }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The Windows system directory without a trailing separator or NUL.
fn system_directory() -> io::Result<Vec<u16>> {
    let needed = unsafe { GetSystemDirectoryW(std::ptr::null_mut(), 0) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; needed as usize];
    let written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), needed) };
    if written == 0 {
        return Err(io::Error::last_os_error());
    }
    if written >= needed {
        return Err(io::Error::other(
            "the Windows system directory changed while it was read",
        ));
    }
    buffer.truncate(written as usize);
    Ok(buffer)
}

/// UTF-16 environment block for `CREATE_UNICODE_ENVIRONMENT`: `Name=Value`
/// entries sorted by name without regard to case, each NUL-terminated, and one
/// more NUL at the end. Names that differ only in case collapse to one entry;
/// the later one in the map's order wins, as it did for `std::process::Command`.
pub(super) fn environment_block(environment: &BTreeMap<String, String>) -> Vec<u16> {
    let mut sorted: BTreeMap<Vec<u16>, (&str, &str)> = BTreeMap::new();
    for (name, value) in environment {
        let key = name.to_uppercase().encode_utf16().collect();
        sorted.insert(key, (name.as_str(), value.as_str()));
    }
    let mut block = Vec::new();
    for (name, value) in sorted.into_values() {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}
