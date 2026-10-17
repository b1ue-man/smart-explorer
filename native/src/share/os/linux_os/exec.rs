#[path = "exec_account.rs"]
mod exec_account;
#[path = "exec_account_unit.rs"]
mod exec_account_unit;
#[path = "exec_binary_copy.rs"]
mod exec_binary_copy;
#[path = "exec_helper_client.rs"]
mod exec_helper_client;
#[path = "exec_helper_config.rs"]
mod exec_helper_config;
#[path = "exec_helper_server.rs"]
mod exec_helper_server;
#[path = "exec_helper_setup.rs"]
mod exec_helper_setup;
#[path = "exec_pam_service.rs"]
mod exec_pam_service;
#[path = "exec_program_search.rs"]
mod exec_program_search;
#[path = "exec_pty.rs"]
mod exec_pty;
#[cfg(all(test, target_os = "linux"))]
#[path = "exec_pty_tests.rs"]
mod exec_pty_tests;
#[path = "exec_remote_account.rs"]
mod exec_remote_account;
#[cfg(debug_assertions)]
#[path = "exec_self_test.rs"]
mod exec_self_test;
#[path = "exec_supervisor.rs"]
mod exec_supervisor;
#[path = "exec_supervisor_stdio.rs"]
mod exec_supervisor_stdio;
#[path = "exec_systemd.rs"]
mod exec_systemd;

pub(crate) use self::exec_helper_setup::{
    exec_accounts_apply, invoking_uid, Elevation, ExecAccountsAction, ExecAccountsOutcome,
};

use std::ffi::OsString;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

use self::exec_account_unit::AccountJob;
use self::exec_binary_copy::runtime_directory;
use self::exec_helper_client::peer_credentials;
use self::exec_systemd::{
    cgroup_populated, manager_connection, require_cgroup_v2, start_unit, stop_unit,
    unit_active_state, unit_control_group, wait_for_unit_pid,
};

use crate::share::exec_helper_protocol::AccountChoices;
use crate::share::exec_platform::StopReason;
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, RunAsAccount, SupervisorCommand, SupervisorEvent,
};
use crate::share::exec_types::{ExecProviderStatus, ExecStart};

const INTERNAL_MODE: &str = "--share-exec-supervisor";
/// The values the Linux supervisor always used: an absolute `$SHELL` or
/// `/bin/sh` as a login shell, no own process group and no intermediate
/// (the transient unit's cgroup contains the whole job).
const LINUX_SPAWN: exec_supervisor::SpawnPolicy<'static> = exec_supervisor::SpawnPolicy {
    default_shell: "/bin/sh",
    shell_from_environment: true,
    shell_option: "-lc",
    own_process_group: false,
    intermediate: None,
};

pub(crate) struct ContainedExec {
    control: Arc<Control>,
    control_group: PathBuf,
    /// The worker's own account: the listener the supervisor connected to.
    socket_path: Option<PathBuf>,
    events: mpsc::Receiver<io::Result<SupervisorEvent>>,
    #[cfg(debug_assertions)]
    supervisor_pid: u32,
}

struct Control {
    unit: JobUnit,
    outbound: mpsc::SyncSender<SupervisorCommand>,
    stopped: AtomicBool,
}

/// Where a job's supervisor runs, and how its unit is stopped and checked.
enum JobUnit {
    /// The worker's own account: a unit in the worker's manager (the user
    /// manager, or the system manager of a root worker).
    Own {
        connection: Connection,
        name: String,
        path: OwnedObjectPath,
    },
    /// Another account: its account unit (PRM-M5, PRM-M6).
    Account(AccountJob),
}

impl ContainedExec {
    /// Prepares the job's unit for the account `exec_platform` prepares the
    /// start for on this thread (`ContainedExec::prepare_as`): none is the
    /// worker's own account, exactly as before PRM-M5; another account gets
    /// its account unit.
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        let Some(account) = crate::share::exec_platform::preparing_account() else {
            return Self::prepare_own(request);
        };
        let (stream, job, control_group, pid) = AccountJob::launch(request, &account)?;
        Self::with_supervisor(stream, JobUnit::Account(job), control_group, None, pid)
    }

    fn prepare_own(request: &ExecStart) -> io::Result<Self> {
        require_cgroup_v2()?;
        let connection = manager_connection()?;
        let directory = runtime_directory()?;
        let socket_path = directory.join(format!("{}.sock", request.exec_id.as_str()));
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let unit_name = format!("smart-explorer-exec-{}.service", request.exec_id.as_str());
        let runtime_usec = request
            .effective_timeout_ms()
            .map(|timeout_ms| timeout_ms.saturating_add(30_000).saturating_mul(1_000));
        if let Err(error) = start_unit(&connection, &unit_name, &socket_path, runtime_usec) {
            let _ = std::fs::remove_file(&socket_path);
            return Err(error);
        }
        let accepted = accept_supervisor(&listener, Instant::now() + Duration::from_secs(15));
        let (stream, credentials) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                let _ = stop_unit(&connection, &unit_name);
                let _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };
        let setup = (|| {
            let expected_uid = unsafe { libc::geteuid() };
            if credentials.uid != expected_uid || credentials.gid != unsafe { libc::getegid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "exec supervisor has the wrong Unix credentials",
                ));
            }
            let unit_path = wait_for_unit_pid(
                &connection,
                &unit_name,
                credentials.pid as u32,
                Instant::now() + Duration::from_secs(10),
            )?;
            let control_group = unit_control_group(&connection, &unit_path)?;
            Ok((unit_path, control_group))
        })();
        let (unit_path, control_group) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                let _ = stop_unit(&connection, &unit_name);
                let _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };
        let unit = JobUnit::Own {
            connection,
            name: unit_name,
            path: unit_path,
        };
        Self::with_supervisor(
            stream,
            unit,
            control_group,
            Some(socket_path),
            credentials.pid as u32,
        )
    }

    /// Connects the worker to the supervisor in `unit`: one thread forwards
    /// the commands, another the events. A failure ends the unit.
    fn with_supervisor(
        stream: UnixStream,
        unit: JobUnit,
        control_group: PathBuf,
        socket_path: Option<PathBuf>,
        #[cfg_attr(not(debug_assertions), allow(unused_variables))] supervisor_pid: u32,
    ) -> io::Result<Self> {
        let (outbound, outbound_rx) = mpsc::sync_channel::<SupervisorCommand>(16);
        let control = Arc::new(Control {
            unit,
            outbound,
            stopped: AtomicBool::new(false),
        });
        match start_plumbing(stream, outbound_rx) {
            Ok(events) => Ok(Self {
                control,
                control_group,
                socket_path,
                events,
                #[cfg(debug_assertions)]
                supervisor_pid,
            }),
            Err(error) => {
                let _ = control.terminate();
                if let Some(path) = &socket_path {
                    let _ = std::fs::remove_file(path);
                }
                Err(error)
            }
        }
    }

    pub(crate) fn configure(&mut self, _request: &ExecStart) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn send(&mut self, command: &SupervisorCommand) -> io::Result<()> {
        self.control.send(command)
    }

    pub(crate) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        match deadline {
            Some(deadline) => self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(recv_timeout_error)?,
            None => self.events.recv().map_err(|_| supervisor_closed())?,
        }
    }

    pub(crate) fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        self.control.terminate()
    }

    pub(crate) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        loop {
            if self.control.finished(&self.control_group)? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} still has a populated cgroup", self.control.unit_name()),
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Control {
    fn send(&self, command: &SupervisorCommand) -> io::Result<()> {
        self.outbound
            .try_send(command.clone())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "exec supervisor input queue is full",
                ),
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "exec supervisor input closed")
                }
            })
    }

    fn terminate(&self) -> io::Result<()> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        // StopUnit is the non-cooperative kill path. It must not wait behind a
        // full supervisor input socket when a silent command is revoked.
        match &self.unit {
            JobUnit::Own {
                connection, name, ..
            } => stop_unit(connection, name),
            JobUnit::Account(job) => job.stop(),
        }
    }

    /// The unit has ended and no process of it is left.
    fn finished(&self, control_group: &Path) -> io::Result<bool> {
        match &self.unit {
            JobUnit::Own {
                connection, path, ..
            } => {
                let active =
                    unit_active_state(connection, path).unwrap_or_else(|_| "inactive".into());
                let populated = cgroup_populated(control_group)?;
                Ok(!populated && matches!(active.as_str(), "inactive" | "failed"))
            }
            JobUnit::Account(job) => job.finished(),
        }
    }

    fn unit_name(&self) -> String {
        match &self.unit {
            JobUnit::Own { name, .. } => name.clone(),
            JobUnit::Account(job) => job.name(),
        }
    }
}

impl Drop for ContainedExec {
    fn drop(&mut self) {
        let _ = self.terminate_all(StopReason::WorkerStopping);
        let _ = self.confirm_empty(Instant::now() + Duration::from_secs(5));
        if let Some(path) = &self.socket_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The two threads between the worker and its supervisor: one writes the
/// commands of `outbound`, one reads the events.
fn start_plumbing(
    stream: UnixStream,
    outbound: mpsc::Receiver<SupervisorCommand>,
) -> io::Result<mpsc::Receiver<io::Result<SupervisorEvent>>> {
    let writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(10)))?;
    let (event_tx, events) = mpsc::sync_channel(32);
    let writer_errors = event_tx.clone();
    std::thread::Builder::new()
        .name("exec-systemd-input".into())
        .spawn(move || {
            let mut writer = writer;
            while let Ok(command) = outbound.recv() {
                if let Err(error) = send_command(&mut writer, &command) {
                    let _ = writer_errors.send(Err(error));
                    break;
                }
            }
        })?;
    std::thread::Builder::new()
        .name("exec-systemd-events".into())
        .spawn(move || {
            let mut reader = stream;
            loop {
                let event = recv_event(&mut reader);
                let terminal = event.is_err();
                if event_tx.send(event).is_err() || terminal {
                    break;
                }
            }
        })?;
    Ok(events)
}

fn recv_timeout_error(error: mpsc::RecvTimeoutError) -> io::Error {
    match error {
        mpsc::RecvTimeoutError::Timeout => {
            io::Error::new(io::ErrorKind::TimedOut, "exec event deadline elapsed")
        }
        mpsc::RecvTimeoutError::Disconnected => supervisor_closed(),
    }
}

fn supervisor_closed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "exec supervisor closed")
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    let elevated = unsafe { libc::geteuid() } == 0;
    let user_label = exec_account::own_account_name();
    match require_cgroup_v2().and_then(|_| manager_connection().map(|_| ())) {
        Ok(()) => ExecProviderStatus {
            available: true,
            provider: "systemd transient cgroup".into(),
            detail: "cgroup v2 and the systemd manager are available".into(),
            elevated,
            user_label,
        },
        Err(error) => ExecProviderStatus {
            available: false,
            provider: "systemd transient cgroup".into(),
            detail: error.to_string(),
            elevated,
            user_label,
        },
    }
}

/// True when this host can give an Exec start a terminal: a pseudo-terminal
/// opens. The pair itself opens in the supervisor inside the job's transient
/// unit, which has no `PrivateDevices`, so `/dev/ptmx` is the same there.
pub(crate) fn terminal_supported() -> bool {
    exec_pty::terminal_supported()
}

/// A root worker, or a worker the account service serves, may start a job
/// as another account (PRM-M5, PRM-M6).
pub(crate) fn run_as_supported() -> bool {
    exec_remote_account::run_as_supported()
}

/// "Ausführen als" of this host with the state of its account service.
pub(crate) fn account_choices() -> AccountChoices {
    exec_remote_account::account_choices()
}

/// Lookup path of a grant with a program list: fixed, never the caller's.
pub(crate) fn program_search() -> crate::share::exec_policy::program_path::ProgramSearch {
    exec_program_search::fixed()
}

/// The grant's run-as account at admission: the checks of every account
/// unit, and for a worker that is not root the account service's list.
pub(crate) fn resolve_account(name: &str) -> io::Result<RunAsAccount> {
    exec_account::resolve_for_start(name)
}

/// The hidden modes of this program on Linux: the supervisors of the
/// worker's own account and of account units, the account service and its
/// setup through `pkexec`.
pub(crate) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    #[cfg(debug_assertions)]
    if let Some(result) = exec_self_test::run_helper_if_requested(arguments) {
        return Some(result);
    }
    let hidden = exec_supervisor_stdio::run_if_requested(arguments, &LINUX_SPAWN)
        .or_else(|| exec_helper_server::run_if_requested(arguments))
        .or_else(|| exec_helper_setup::run_if_requested(arguments));
    if hidden.is_some() {
        return hidden;
    }
    if arguments.len() != 2 || arguments[0] != INTERNAL_MODE {
        return None;
    }
    let socket = PathBuf::from(&arguments[1]);
    let result = (|| {
        let directory = runtime_directory()?;
        if !socket.starts_with(&directory)
            || socket.extension().and_then(|v| v.to_str()) != Some("sock")
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid exec supervisor socket",
            ));
        }
        let stream = std::os::unix::net::UnixStream::connect(&socket)?;
        // The connected Unix socket no longer needs a directory entry. Unlink
        // it in the independently supervised process so a SIGKILL of the
        // worker cannot leave a stale path behind.
        std::fs::remove_file(&socket)?;
        exec_supervisor::run(stream, &LINUX_SPAWN)
    })();
    Some(result)
}

#[cfg(debug_assertions)]
pub(crate) fn run_extended_self_test() -> io::Result<()> {
    exec_self_test::run()
}

fn accept_supervisor(
    listener: &std::os::unix::net::UnixListener,
    deadline: Instant,
) -> io::Result<(std::os::unix::net::UnixStream, libc::ucred)> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let credentials = peer_credentials(&stream)?;
                return Ok((stream, credentials));
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}
