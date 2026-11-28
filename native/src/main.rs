#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    quiet_critical_errors();
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(result) = smart_explorer::mount::run_host_if_requested(&arguments) {
        exit_mount_host(result);
    }
    smart_explorer::run_gui()
}

/// No system dialog for a missing medium (an empty card reader, a removed
/// stick): the process gets the error instead and reports it (U18). Set once
/// for the whole process before any thread starts; the bits already set
/// stay. Child processes inherit the mode, as Windows documents for
/// `SetErrorMode`.
#[cfg(windows)]
fn quiet_critical_errors() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        GetErrorMode, SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX,
    };
    // SAFETY: process-wide flags only; no pointers are involved.
    unsafe {
        SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

fn exit_mount_host(result: Result<(), String>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("smart-explorer: internal mount host failed: {error}");
            std::process::exit(1)
        }
    }
}
