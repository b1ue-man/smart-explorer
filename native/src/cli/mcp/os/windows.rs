use std::fs::File;
use std::io;
use std::os::windows::io::FromRawHandle;
use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub(super) fn protect_stdout() -> io::Result<File> {
    // SAFETY: duplicates the process's own standard output handle and swaps
    // the standard output slot; Rust's stdout queries the slot per write.
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if output.is_null() || output == INVALID_HANDLE_VALUE {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the process has no standard output for the protocol",
            ));
        }
        let process = GetCurrentProcess();
        let mut protocol: HANDLE = std::ptr::null_mut();
        if DuplicateHandle(
            process,
            output,
            process,
            &mut protocol,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let errors = GetStdHandle(STD_ERROR_HANDLE);
        if !errors.is_null() && errors != INVALID_HANDLE_VALUE {
            SetStdHandle(STD_OUTPUT_HANDLE, errors);
        }
        Ok(File::from_raw_handle(protocol as _))
    }
}

pub(super) fn configure_child(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    // An agent host may run this server without a console; a console child
    // would otherwise open a visible window for every call.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// A windowless child has its own hidden console, which this process cannot
/// send Ctrl+C or Ctrl+Break to; it is terminated instead.
pub(super) fn request_child_stop(_child: &std::process::Child) -> bool {
    false
}
