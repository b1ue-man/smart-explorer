//! Windows: UAC through `ShellExecuteExW` „runas“ of `se.exe elevated …`;
//! the helper's streams travel over two named pipes (`windows/pipe.rs`), its
//! program is held unchanged in between (`windows/image.rs`), process trees
//! are job objects (`windows/job.rs`).
#[path = "windows/credui.rs"]
mod credui;
#[path = "windows/image.rs"]
mod image;
#[path = "windows/job.rs"]
mod job;
#[path = "windows/pipe.rs"]
mod pipe;
#[path = "windows/token.rs"]
mod token;

use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::ptr::null;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::child::ElevatedChild;
use super::framed::{self, Launched};
use super::helper_args::{self, HelperCall, PipeBinding};
use super::types::{ElevateError, ElevatedCommand, ElevatedSpec, Elevation};

pub use job::Tree;

pub fn is_elevated() -> bool {
    token::is_elevated()
}

/// `se.exe` beside this program (the desktop app has no `se elevated`).
fn se_program() -> Result<PathBuf, ElevateError> {
    let current = std::env::current_exe()
        .map_err(|error| ElevateError::Failed(format!("cannot find this program: {error}")))?;
    let is_se = current
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("se.exe"));
    let se = if is_se {
        current
    } else {
        current.with_file_name("se.exe")
    };
    if !se.is_file() {
        return Err(ElevateError::Failed(format!(
            "{} is missing; the elevated helper is part of the se program",
            se.display()
        )));
    }
    Ok(se)
}

struct Com;

impl Com {
    fn init() -> Result<Self, ElevateError> {
        // SAFETY: COM for ShellExecuteEx on this thread, undone by Drop.
        let result = unsafe {
            CoInitializeEx(
                null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        };
        if result < 0 {
            return Err(ElevateError::Failed(format!("COM for UAC: {result:#x}")));
        }
        Ok(Self)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: pairs the successful CoInitializeEx of `init`.
        unsafe { CoUninitialize() };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// `ShellExecuteExW` „runas“ of `program` with `args`; the process handle.
fn run_as_admin(program: &std::path::Path, args: &[String]) -> Result<OwnedHandle, ElevateError> {
    let _com = Com::init()?;
    let parameters = helper_args::windows_command_line(args);
    if parameters.encode_utf16().count() > 32_000 {
        return Err(ElevateError::Failed(
            "this command is too long for an elevated start; write it to a script file and run that file"
                .into(),
        ));
    }
    let file = wide(&program.to_string_lossy());
    let parameters = wide(&parameters);
    let verb = wide("runas");
    // SAFETY: a zeroed structure whose pointers live until the call returns.
    let mut execute: SHELLEXECUTEINFOW = unsafe { zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = file.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    execute.nShow = SW_HIDE;
    // SAFETY: the structure above.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        // SAFETY: the error of the failed call on this thread.
        let error = unsafe { GetLastError() };
        if error == ERROR_CANCELLED {
            return Err(ElevateError::Declined);
        }
        return Err(ElevateError::NotAllowed(format!(
            "Windows refused the elevated start: {}",
            io::Error::from_raw_os_error(error as i32)
        )));
    }
    if execute.hProcess.is_null() {
        return Err(ElevateError::Failed(
            "Windows returned no elevated process".into(),
        ));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS hands the process handle to the caller.
    Ok(unsafe { OwnedHandle::from_raw_handle(execute.hProcess) })
}

pub fn spawn_elevated(spec: ElevatedSpec) -> Result<ElevatedChild, ElevateError> {
    let se = se_program()?;
    let image = image::LockedImage::at(se)
        .map_err(|error| ElevateError::Failed(format!("cannot hold the se program: {error}")))?;
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|error| ElevateError::Failed(error.to_string()))?;
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let name = format!(r"\\.\pipe\se-elevate-{nonce}");
    let pipes = pipe::create(&name).map_err(|error| {
        ElevateError::Failed(format!("cannot create the elevation pipes: {error}"))
    })?;
    let call = HelperCall {
        spec,
        pipe: Some(PipeBinding {
            name,
            server_pid: std::process::id(),
        }),
        image_hash: Some(image.hash.clone()),
    };
    let mut args = vec!["elevated".to_string()];
    args.extend(helper_args::build("exec", &call));
    let process = run_as_admin(&image.path, &args)?;
    let raw = process.as_raw_handle() as HANDLE;
    // SAFETY: the process handle `process` owns.
    let pid = unsafe { GetProcessId(raw) };
    pipes
        .accept(raw, pid)
        .map_err(|error| ElevateError::Failed(format!("elevated helper: {error}")))?;
    drop(image);
    let mut reader = pipes.from_helper;
    if !framed::await_ready(&mut reader) {
        return Err(ElevateError::Failed(
            "the elevated helper ended before it was ready".into(),
        ));
    }
    let writer = pipes.to_helper;
    let held = process;
    Ok(framed::start(
        Launched {
            reader: Box::new(reader),
            writer: Box::new(writer),
            // An unelevated process may not end the elevated helper; the
            // cancel frame and the closing pipes end it.
            abort: Box::new(|| {}),
            reap: Box::new(move || {
                // SAFETY: the helper's process handle, kept until here.
                unsafe { WaitForSingleObject(held.as_raw_handle() as HANDLE, INFINITE) };
            }),
        },
        Elevation::Uac,
    ))
}

pub fn confirm_user_presence(reason: &str) -> Result<(), ElevateError> {
    let se = se_program()?;
    let args = vec![
        "elevated".to_string(),
        "confirm".to_string(),
        "--reason".to_string(),
        reason.to_string(),
    ];
    let process = run_as_admin(&se, &args)?;
    let raw = process.as_raw_handle() as HANDLE;
    let mut code = 1u32;
    // SAFETY: the helper's process handle; waits for its end.
    let ended = unsafe { WaitForSingleObject(raw, INFINITE) } == WAIT_OBJECT_0
        && unsafe { GetExitCodeProcess(raw, &mut code) } != 0;
    if ended && code == 0 {
        Ok(())
    } else {
        Err(ElevateError::Failed(format!(
            "the confirmation helper ended with {code}"
        )))
    }
}

pub fn ask_password(
    caption: &str,
    message: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, ElevateError> {
    credui::ask_password(caption, message)
}

/// The helper's channel: the pipes of the caller, checked against its
/// process id, and this program's SHA-256 against the requested one.
#[allow(clippy::type_complexity)]
pub fn helper_transport(
    call: &HelperCall,
) -> Result<(Box<dyn Read + Send>, Box<dyn Write + Send>), String> {
    let binding = call
        .pipe
        .as_ref()
        .ok_or_else(|| "se elevated exec needs --pipe on Windows".to_string())?;
    let expected = call
        .image_hash
        .as_ref()
        .ok_or_else(|| "se elevated exec needs --image-hash on Windows".to_string())?;
    let own = std::env::current_exe().map_err(|error| error.to_string())?;
    let actual = image::LockedImage::at(own).map_err(|error| error.to_string())?;
    if &actual.hash != expected {
        return Err("se elevated: this program is not the one the caller asked for".into());
    }
    let (from_caller, to_caller) = pipe::open_helper_ends(&binding.name, binding.server_pid)
        .map_err(|error| format!("se elevated: {error}"))?;
    Ok((Box::new(from_caller), Box::new(to_caller)))
}

pub fn direct_command(command: &ElevatedCommand) -> io::Result<Command> {
    let mut started = match command {
        ElevatedCommand::Argv { program, args } => {
            let mut started = Command::new(program);
            started.args(args);
            started
        }
        ElevatedCommand::CmdLine(line) => {
            let mut started = Command::new("cmd.exe");
            started.args(["/D", "/S", "/C"]).raw_arg(line);
            started
        }
    };
    started.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    Ok(started)
}

pub fn exit_signal(_status: &ExitStatus) -> Option<i32> {
    None
}
