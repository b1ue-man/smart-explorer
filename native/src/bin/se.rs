#[cfg(windows)]
mod se_path_windows;

fn main() {
    #[cfg(windows)]
    quiet_critical_errors();
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(result) = smart_explorer::run_login_service_if_requested(&arguments) {
        match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("se: login service: {error}");
                std::process::exit(1);
            }
        }
    }
    // P4: `pam_exec` of xrdp-sesman runs `se --rdp-xrdp-login` as root; it
    // answers with its exit code alone, before any other setup of this program.
    if let Some(code) = smart_explorer::rdp::run_xrdp_login_if_requested(&arguments) {
        std::process::exit(code);
    }
    if let Some(result) = smart_explorer::mcp_launcher::runtime_contract_probe(&arguments) {
        match result {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("se: {error}");
                std::process::exit(1);
            }
        }
    }
    let _mcp_admission = match smart_explorer::mcp_launcher::admit_cli_process() {
        Ok(admission) => admission,
        Err(error) => {
            eprintln!("se: {error}");
            std::process::exit(1);
        }
    };
    if let Some(result) = smart_explorer::share::run_exec_supervisor_if_requested(&arguments) {
        exit_internal_portable(result);
    }
    if let Some(result) = smart_explorer::mount::run_host_if_requested(&arguments) {
        exit_mount_host(result);
    }
    if let Some(result) = smart_explorer::net::run_uplink_helper_if_requested(&arguments) {
        exit_uplink_helper(result);
    }
    #[cfg(debug_assertions)]
    if is_internal_invocation(&arguments, "--share-exec-platform-self-test") {
        exit_internal_portable(smart_explorer::share::run_exec_platform_self_test());
    }
    if is_internal_invocation(&arguments, "--sync-guardian") {
        smart_explorer::daemon::run_guardian();
        return;
    }
    if smart_explorer::daemon::is_drive_restore_invocation(&arguments) {
        smart_explorer::daemon::note_daemon_start_args(&arguments);
        smart_explorer::daemon::run_daemon();
        return;
    }
    if is_internal_invocation(&arguments, "--sync-daemon") {
        smart_explorer::daemon::run_daemon();
        return;
    }
    #[cfg(windows)]
    if is_internal_invocation(&arguments, "--install-cli-path") {
        exit_internal(se_path_windows::register());
    }
    #[cfg(windows)]
    if is_internal_invocation(&arguments, "--uninstall-cli-path") {
        exit_internal(se_path_windows::unregister());
    }
    if let Some(code) = smart_explorer::run_screen_host_worker_if_requested(&arguments) {
        std::process::exit(code);
    }
    std::process::exit(smart_explorer::cli::run());
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
            eprintln!("se: internal mount host failed: {error}");
            std::process::exit(1)
        }
    }
}

fn exit_uplink_helper(result: std::io::Result<()>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("se: LAN uplink helper failed: {error}");
            std::process::exit(1)
        }
    }
}

fn exit_internal_portable(result: std::io::Result<()>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("se: internal remote-exec process failed: {error}");
            std::process::exit(1)
        }
    }
}

#[cfg(windows)]
fn exit_internal(result: std::io::Result<()>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("se: Windows CLI path registration failed: {error}");
            std::process::exit(1)
        }
    }
}

fn is_internal_invocation(arguments: &[std::ffi::OsString], command: &str) -> bool {
    arguments.len() == 1 && arguments[0] == std::ffi::OsStr::new(command)
}

#[cfg(test)]
mod tests {
    use super::is_internal_invocation;

    #[cfg(windows)]
    #[test]
    fn removable_task_missing_medium_raises_no_dialog() {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX,
        };
        let before = unsafe { GetErrorMode() };
        super::quiet_critical_errors();
        let after = unsafe { GetErrorMode() };
        let quiet = SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX;
        assert_eq!(after & quiet, quiet);
        assert_eq!(after & before, before, "bits set before stay set");
    }

    #[test]
    fn daemon_mode_requires_the_only_exact_argument() {
        assert!(is_internal_invocation(
            &["--sync-daemon".into()],
            "--sync-daemon"
        ));
        assert!(!is_internal_invocation(
            &["exec".into(), "--sync-daemon".into()],
            "--sync-daemon"
        ));
        assert!(!is_internal_invocation(
            &["--sync-daemon".into(), "extra".into()],
            "--sync-daemon"
        ));
    }
}
