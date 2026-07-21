#[cfg(windows)]
mod se_path_windows;

fn main() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(result) = smart_explorer::share::run_exec_supervisor_if_requested(&arguments) {
        exit_internal_portable(result);
    }
    if let Some(result) = smart_explorer::mount::run_host_if_requested(&arguments) {
        exit_mount_host(result);
    }
    #[cfg(debug_assertions)]
    if is_internal_invocation(&arguments, "--share-exec-platform-self-test") {
        exit_internal_portable(smart_explorer::share::run_exec_platform_self_test());
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
    std::process::exit(smart_explorer::cli::run());
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
