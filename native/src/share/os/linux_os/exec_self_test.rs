use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use zbus::zvariant::OwnedObjectPath;

use crate::share::exec_supervisor_protocol::{
    environment_for, SupervisorCommand, SupervisorEvent, SupervisorStart,
};
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

use super::exec_systemd::{
    cgroup_populated, manager_connection, reset_failed_unit, start_runtime_test_unit, stop_unit,
    unit_active_state, unit_control_group, unit_result, unit_runtime_max_usec, wait_for_unit_pid,
};
use super::{ContainedExec, StopReason};

const FORK_HELPER_MODE: &str = "--share-exec-linux-fork-containment-probe";
const FORK_READY: &[u8] = b"SE-LINUX-FORKS-READY\n";
const FORK_WORKERS: usize = 8;
const RUNTIME_BACKSTOP_USEC: u64 = 2_000_000;

pub(super) fn run_helper_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    (arguments.len() == 1 && arguments[0] == FORK_HELPER_MODE).then(run_fork_helper)
}

pub(super) fn run() -> io::Result<()> {
    run_fork_tree_stop_test()?;
    run_supervisor_sigkill_test()?;
    run_runtime_backstop_test()
}

fn run_fork_tree_stop_test() -> io::Result<()> {
    let mut process = start_fork_probe()?;
    wait_for_fork_ready(&mut process)?;
    require_cgroup_processes(&process, FORK_WORKERS + 2)?;
    process.terminate_all(StopReason::Cancelled)?;
    process.confirm_empty(Instant::now() + Duration::from_secs(10))
}

fn run_supervisor_sigkill_test() -> io::Result<()> {
    let mut process = start_fork_probe()?;
    wait_for_fork_ready(&mut process)?;
    require_cgroup_processes(&process, FORK_WORKERS + 2)?;
    if unsafe { libc::kill(process.supervisor_pid as i32, libc::SIGKILL) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Do not call StopUnit here: systemd must react to direct death of the
    // supervisor and drain every double-forked descendant itself.
    process.confirm_empty(Instant::now() + Duration::from_secs(10))
}

fn start_fork_probe() -> io::Result<ContainedExec> {
    let executable = std::env::current_exe()?
        .into_os_string()
        .into_string()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-Unicode test executable"))?;
    let request = ExecStart {
        exec_id: ExecId::generate()?,
        command: ExecCommand::Argv {
            program: executable,
            args: vec![FORK_HELPER_MODE.into()],
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: None,
        max_output_bytes: None,
    };
    request.validate()?;
    let environment = environment_for(&request);
    let mut process = ContainedExec::prepare(&request)?;
    process.configure(&request)?;
    process.send(&SupervisorCommand::Start(SupervisorStart {
        request,
        environment,
    }))?;
    Ok(process)
}

fn wait_for_fork_ready(process: &mut ContainedExec) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = Vec::new();
    let mut started = false;
    loop {
        match process.next_event(Some(deadline))? {
            SupervisorEvent::Started { .. } => started = true,
            SupervisorEvent::Stdout(bytes) => {
                output.extend(bytes);
                if started
                    && output
                        .windows(FORK_READY.len())
                        .any(|window| window == FORK_READY)
                {
                    return Ok(());
                }
            }
            SupervisorEvent::RootExited(exit) | SupervisorEvent::Exited(exit) => {
                return Err(io::Error::other(format!(
                    "fork helper exited before readiness: {exit:?}"
                )));
            }
            SupervisorEvent::Error(message) => return Err(io::Error::other(message)),
            SupervisorEvent::Stderr(_) => {}
        }
    }
}

fn require_cgroup_processes(process: &ContainedExec, minimum: usize) -> io::Result<()> {
    let pids = std::fs::read_to_string(process.control_group.join("cgroup.procs"))?;
    let count = pids.lines().filter(|line| !line.trim().is_empty()).count();
    if count < minimum {
        return Err(io::Error::other(format!(
            "fork probe cgroup contains {count} processes; expected at least {minimum}"
        )));
    }
    Ok(())
}

fn run_fork_helper() -> io::Result<()> {
    let mut first_generation = Vec::with_capacity(FORK_WORKERS);
    for _ in 0..FORK_WORKERS {
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            fork_grandchild_and_exit();
        }
        first_generation.push(pid);
    }
    for pid in first_generation {
        let mut status = 0;
        if unsafe { libc::waitpid(pid, &mut status, 0) } != pid
            || !libc::WIFEXITED(status)
            || libc::WEXITSTATUS(status) != 0
        {
            return Err(io::Error::other("double-fork worker failed"));
        }
    }
    std::io::stdout().write_all(FORK_READY)?;
    std::io::stdout().flush()?;
    loop {
        unsafe { libc::pause() };
    }
}

fn fork_grandchild_and_exit() -> ! {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe { libc::_exit(111) };
    }
    if pid > 0 {
        unsafe { libc::_exit(0) };
    }
    unsafe {
        libc::setsid();
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        loop {
            libc::pause();
        }
    }
}

fn run_runtime_backstop_test() -> io::Result<()> {
    let id = ExecId::generate()?;
    let directory = super::runtime_directory()?;
    let socket_path = directory.join(format!("runtime-{}.sock", id.as_str()));
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let connection = manager_connection()?;
    let unit_name = format!("smart-explorer-exec-runtime-{}.service", id.as_str());
    let backstop_started = Instant::now();
    if let Err(error) =
        start_runtime_test_unit(&connection, &unit_name, &socket_path, RUNTIME_BACKSTOP_USEC)
    {
        let _ = std::fs::remove_file(&socket_path);
        return Err(error);
    }
    let mut cleanup_unit: Option<(OwnedObjectPath, PathBuf)> = None;
    let result = (|| {
        let (stream, credentials) =
            super::accept_supervisor(&listener, Instant::now() + Duration::from_secs(15))?;
        if credentials.uid != unsafe { libc::geteuid() }
            || credentials.gid != unsafe { libc::getegid() }
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "RuntimeMaxUSec probe supervisor has the wrong credentials",
            ));
        }
        let unit_path = wait_for_unit_pid(
            &connection,
            &unit_name,
            credentials.pid as u32,
            Instant::now() + Duration::from_secs(10),
        )?;
        let control_group = unit_control_group(&connection, &unit_path)?;
        cleanup_unit = Some((unit_path.clone(), control_group.clone()));
        let configured_runtime = unit_runtime_max_usec(&connection, &unit_path)?;
        if configured_runtime != RUNTIME_BACKSTOP_USEC {
            return Err(io::Error::other(format!(
                "RuntimeMaxUSec is {configured_runtime}; expected {RUNTIME_BACKSTOP_USEC}"
            )));
        }
        if !cgroup_populated(&control_group)? {
            return Err(io::Error::other(
                "RuntimeMaxUSec probe never populated its cgroup",
            ));
        }
        let deadline = backstop_started + Duration::from_secs(10);
        loop {
            let active = unit_active_state(&connection, &unit_path)?;
            let service_result = unit_result(&connection, &unit_path)?;
            if !cgroup_populated(&control_group)?
                && matches!(active.as_str(), "inactive" | "failed")
            {
                if active != "failed" || service_result != "timeout" {
                    return Err(io::Error::other(
                        format!(
                            "RuntimeMaxUSec probe ended as {active}/{service_result}, not failed/timeout"
                        ),
                    ));
                }
                drop(stream);
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RuntimeMaxUSec did not drain the hung supervisor cgroup",
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    let cleanup =
        cleanup_runtime_unit(&connection, &unit_name, cleanup_unit.as_ref(), &socket_path);
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(io::Error::other(format!(
            "{error}; RuntimeMaxUSec cleanup also failed: {cleanup}"
        ))),
    }
}

fn cleanup_runtime_unit(
    connection: &zbus::blocking::Connection,
    unit_name: &str,
    unit: Option<&(OwnedObjectPath, PathBuf)>,
    socket_path: &Path,
) -> io::Result<()> {
    let mut failures = Vec::new();
    if let Err(error) = stop_unit(connection, unit_name) {
        failures.push(format!("StopUnit: {error}"));
    }
    if let Some((unit_path, control_group)) = unit {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let active =
                unit_active_state(connection, unit_path).unwrap_or_else(|_| "inactive".to_owned());
            match cgroup_populated(control_group) {
                Ok(false) if matches!(active.as_str(), "inactive" | "failed") => break,
                Ok(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(_) => {
                    failures.push("transient unit cgroup did not drain during cleanup".into());
                    break;
                }
                Err(error) => {
                    failures.push(format!("cgroup cleanup check: {error}"));
                    break;
                }
            }
        }
    }
    if let Err(error) = reset_failed_unit(connection, unit_name) {
        failures.push(format!("ResetFailedUnit: {error}"));
    }
    match std::fs::remove_file(socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!("socket cleanup: {error}")),
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(failures.join("; ")))
    }
}
