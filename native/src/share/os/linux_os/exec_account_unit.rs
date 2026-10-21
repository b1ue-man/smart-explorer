//! The unit an Exec job runs in when its grant names another account than
//! the worker's (PRM-M5, contract D.2.6): a transient service in the system
//! manager with `User=` the account, the PAM service `smart-explorer-exec`,
//! `WorkingDirectory=-~` or the requested directory, and the supervisor's
//! connection as standard input and output (`--share-exec-supervisor-stdio`).
//! The supervisor runs as the account from its first instruction on; nothing
//! switches accounts in a forked child (`Type=notify`: the supervisor says
//! `READY=1` once it runs, see `account_unit_properties`). A root worker starts the unit
//! itself; the account service starts it for a worker that is not root
//! (PRM-M6). A unit that failed to start stays loaded until its starter has
//! read why (`CollectMode=inactive`); every ended unit is reset.
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::share::exec_helper_protocol::{account_start_error, ACCOUNT_UNAVAILABLE};
use crate::share::exec_supervisor_protocol::RunAsAccount;
use crate::share::exec_types::ExecStart;

use super::exec_account;
use super::exec_binary_copy;
use super::exec_helper_client::HelperSession;
use super::exec_pam_service::{self, PAM_SERVICE};
use super::exec_supervisor_stdio::STDIO_MODE;
use super::exec_systemd::{self, cgroup_populated, require_cgroup_v2};

/// Prefix of every job unit (`smart-explorer-exec-<id>.service`).
pub(super) const UNIT_PREFIX: &str = "smart-explorer-exec-";
const ACCOUNT_COLLECT_MODE: &str = "inactive";
/// `TimeoutStartUSec` of the unit (15 s) and a margin.
const START_TIMEOUT: Duration = Duration::from_secs(17);
/// Exit statuses systemd gives a unit whose start failed before `exec`.
const EXIT_CHDIR: i32 = 200;
const EXIT_EXEC: i32 = 203;
const EXIT_USER: i32 = 217;
const EXIT_PAM: i32 = 224;

/// The account properties of a job unit.
pub(super) fn account_properties(
    account: &str,
    working_directory: &str,
    stdio: BorrowedFd<'_>,
) -> io::Result<Vec<(&'static str, OwnedValue)>> {
    Ok(vec![
        ("User", exec_systemd::string_value(account)),
        ("PAMName", exec_systemd::string_value(PAM_SERVICE)),
        (
            "WorkingDirectory",
            exec_systemd::string_value(working_directory),
        ),
        (
            "StandardInputFileDescriptor",
            exec_systemd::fd_value(stdio)?,
        ),
        (
            "StandardOutputFileDescriptor",
            exec_systemd::fd_value(stdio)?,
        ),
    ])
}

/// Every property of an account unit: those of every job unit with the
/// supervisor `program --share-exec-supervisor-stdio`, then `account`.
pub(super) fn account_unit_properties(
    program: &Path,
    account: Vec<(&'static str, OwnedValue)>,
    runtime_usec: Option<u64>,
) -> io::Result<Vec<(&'static str, OwnedValue)>> {
    let program = program
        .to_str()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the supervisor program path is not UTF-8",
            )
        })?
        .to_owned();
    let exec = vec![(program.clone(), vec![program, STDIO_MODE.into()], false)];
    let mut properties = exec_systemd::job_properties(
        "Smart Explorer remote execution as another account",
        exec,
        runtime_usec,
        ACCOUNT_COLLECT_MODE,
    )?;
    // `Type=exec` would never finish here: with `PAMName=` the service
    // manager forks a PAM session process that keeps the descriptor which
    // signals the `execve()` open (fixed in systemd 256, not in 249 or 255),
    // so the start job times out after `TimeoutStartUSec`. The supervisor
    // reports its readiness itself (`READY=1`, `exec_supervisor_stdio.rs`).
    for (name, value) in &mut properties {
        if *name == "Type" {
            *value = exec_systemd::string_value("notify");
        }
    }
    properties.extend(account);
    Ok(properties)
}

/// `WorkingDirectory=` of a job: without a requested directory the
/// account's home, where a missing home is no error and the job starts in
/// `/`, as with sshd; a requested directory has to exist. A relative one
/// counts from `/`, as in the units of the worker's own account.
pub(super) fn working_directory(cwd: Option<&str>) -> String {
    match cwd {
        None => "-~".into(),
        Some(cwd) if cwd.starts_with('/') => cwd.into(),
        Some(cwd) => format!("/{cwd}"),
    }
}

/// What starts an account unit.
pub(super) struct AccountUnitStart<'a> {
    pub(super) account: &'a str,
    pub(super) cwd: Option<&'a str>,
    pub(super) program: &'a Path,
    pub(super) runtime_usec: Option<u64>,
    /// The supervisor's end of its connection; systemd keeps a duplicate.
    pub(super) stdio: BorrowedFd<'a>,
}

/// Whether a unit still counts as running, and whether its control group
/// has no process left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UnitStatus {
    pub(super) active: bool,
    pub(super) empty: bool,
}

impl UnitStatus {
    pub(super) fn finished(self) -> bool {
        !self.active && self.empty
    }
}

/// One account unit in the system manager.
pub(super) struct AccountUnit {
    connection: Connection,
    name: String,
    path: OwnedObjectPath,
    control_group: PathBuf,
    main_pid: u32,
}

impl AccountUnit {
    /// Starts the unit `name` and returns once its supervisor runs as the
    /// account, or with the reason systemd or PAM refused it.
    pub(super) fn start(
        connection: Connection,
        name: String,
        start: &AccountUnitStart<'_>,
    ) -> io::Result<Self> {
        let account =
            account_properties(start.account, &working_directory(start.cwd), start.stdio)?;
        let properties = account_unit_properties(start.program, account, start.runtime_usec)?;
        exec_systemd::start_transient_unit(&connection, &name, properties)?;
        let running = wait_until_running(&connection, &name, start).and_then(|(path, pid)| {
            let control_group = exec_systemd::unit_control_group(&connection, &path)?;
            Ok((path, pid, control_group))
        });
        match running {
            Ok((path, main_pid, control_group)) => Ok(Self {
                connection,
                name,
                path,
                control_group,
                main_pid,
            }),
            Err(error) => {
                let _ = exec_systemd::stop_unit(&connection, &name);
                let _ = exec_systemd::reset_failed_unit(&connection, &name);
                Err(error)
            }
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn control_group(&self) -> &Path {
        &self.control_group
    }

    /// The supervisor's pid.
    pub(super) fn main_pid(&self) -> u32 {
        self.main_pid
    }

    /// Ends every process of the unit (`KillSignal=SIGKILL` to the whole
    /// control group).
    pub(super) fn stop(&self) -> io::Result<()> {
        exec_systemd::stop_unit(&self.connection, &self.name)
    }

    pub(super) fn status(&self) -> io::Result<UnitStatus> {
        let state = exec_systemd::unit_active_state(&self.connection, &self.path)
            .unwrap_or_else(|_| "inactive".into());
        Ok(UnitStatus {
            active: !matches!(state.as_str(), "inactive" | "failed"),
            empty: !cgroup_populated(&self.control_group)?,
        })
    }

    /// Unloads the ended unit; `CollectMode=inactive` keeps a failed one.
    pub(super) fn collect(&self) {
        let _ = exec_systemd::reset_failed_unit(&self.connection, &self.name);
    }
}

/// The account unit of one job: started by this root worker (PRM-M5), or
/// by the account service for a worker that is not root (PRM-M6).
pub(super) enum AccountJob {
    Worker(AccountUnit),
    Service(Mutex<HelperSession>),
}

impl AccountJob {
    /// Starts the account unit of `request`. Returns the worker's end of the
    /// supervisor's connection, the job, its control group (empty where the
    /// account service watches it) and the supervisor's pid (0 where unknown).
    pub(super) fn launch(
        request: &ExecStart,
        account: &RunAsAccount,
    ) -> io::Result<(UnixStream, Self, PathBuf, u32)> {
        // SAFETY: `geteuid` only reads this process's id.
        if unsafe { libc::geteuid() } == 0 {
            let (stream, unit) = launch_for_root_worker(request, account)?;
            let control_group = unit.control_group().to_path_buf();
            let pid = unit.main_pid();
            return Ok((stream, Self::Worker(unit), control_group, pid));
        }
        require_cgroup_v2()?;
        let (stream, supervisor) = UnixStream::pair()?;
        let session = HelperSession::start(&account.name, request.cwd.as_deref(), &supervisor)?;
        drop(supervisor);
        Ok((
            stream,
            Self::Service(Mutex::new(session)),
            PathBuf::new(),
            0,
        ))
    }

    pub(super) fn stop(&self) -> io::Result<()> {
        match self {
            Self::Worker(unit) => unit.stop(),
            Self::Service(session) => lock(session)?.stop(),
        }
    }

    /// The unit has ended and no process of it is left; an ended unit of
    /// this worker is reset.
    pub(super) fn finished(&self) -> io::Result<bool> {
        match self {
            Self::Worker(unit) => {
                let finished = unit.status()?.finished();
                if finished {
                    unit.collect();
                }
                Ok(finished)
            }
            Self::Service(session) => {
                let (active, empty) = lock(session)?.status()?;
                Ok(!active && empty)
            }
        }
    }

    pub(super) fn name(&self) -> String {
        match self {
            Self::Worker(unit) => unit.name().to_string(),
            Self::Service(session) => lock(session)
                .map(|session| session.name().to_string())
                .unwrap_or_default(),
        }
    }
}

fn lock(session: &Mutex<HelperSession>) -> io::Result<MutexGuard<'_, HelperSession>> {
    session
        .lock()
        .map_err(|_| io::Error::other("the account service session lock is poisoned"))
}

/// A root worker's own start of an account unit (PRM-M5): the account passes
/// the checks of every account unit and is still the one the admission
/// resolved, the PAM service file exists, and the program is trusted and
/// holds this worker's image. The worker keeps one end of a socket pair;
/// the unit gets the other.
fn launch_for_root_worker(
    request: &ExecStart,
    account: &RunAsAccount,
) -> io::Result<(UnixStream, AccountUnit)> {
    require_cgroup_v2()?;
    let checked = exec_account::check_account(&account.name)
        .map_err(|refusal| refusal.into_error(&account.name))?;
    if (checked.uid, checked.gid) != (account.uid, account.gid) {
        return Err(account_start_error(
            ACCOUNT_UNAVAILABLE,
            format!(
                "account {} changed while its start was checked",
                account.name
            ),
        ));
    }
    exec_pam_service::ensure_installed()?;
    let program = exec_binary_copy::program_for_units()?;
    let connection = Connection::system().map_err(exec_systemd::eio)?;
    let (worker, supervisor) = UnixStream::pair()?;
    let name = format!("{UNIT_PREFIX}{}.service", request.exec_id.as_str());
    let runtime_usec = request
        .effective_timeout_ms()
        .map(|timeout_ms| timeout_ms.saturating_add(30_000).saturating_mul(1_000));
    let unit = AccountUnit::start(
        connection,
        name,
        &AccountUnitStart {
            account: &account.name,
            cwd: request.cwd.as_deref(),
            program: &program,
            runtime_usec,
            stdio: supervisor.as_fd(),
        },
    )?;
    drop(supervisor);
    Ok((worker, unit))
}

fn wait_until_running(
    connection: &Connection,
    name: &str,
    start: &AccountUnitStart<'_>,
) -> io::Result<(OwnedObjectPath, u32)> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let Some(path) = exec_systemd::loaded_unit(connection, name)? else {
            return Err(account_start_error(
                ACCOUNT_UNAVAILABLE,
                format!("the unit for account {} ended before it ran", start.account),
            ));
        };
        let state = exec_systemd::unit_active_state(connection, &path)?;
        let ended = match state.as_str() {
            "active" => {
                let pid = exec_systemd::unit_main_pid(connection, &path)?;
                if pid > 0 {
                    return Ok((path, pid));
                }
                false
            }
            "failed" => true,
            "inactive" | "deactivating" => exec_systemd::unit_was_started(connection, &path)?,
            _ => false,
        };
        if ended {
            return Err(start_failure(connection, &path, start));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "the unit for account {} did not start within {} s",
                    start.account,
                    START_TIMEOUT.as_secs()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The reason systemd gives for a failed start, in the words of C6.
fn start_failure(
    connection: &Connection,
    path: &OwnedObjectPath,
    start: &AccountUnitStart<'_>,
) -> io::Error {
    let (result, status) =
        exec_systemd::unit_exit_status(connection, path).unwrap_or_else(|_| ("unknown".into(), 0));
    let account = start.account;
    let detail = match status {
        EXIT_PAM => format!(
            "PAM ({PAM_SERVICE}) refused account {account}: it is expired or locked, needs a \
             password change, or the PAM rules of this host reject it"
        ),
        EXIT_USER => format!("account {account} does not exist on this host"),
        EXIT_CHDIR => format!(
            "the working directory {} does not exist or account {account} cannot enter it",
            start.cwd.unwrap_or("~")
        ),
        EXIT_EXEC => format!(
            "{} could not run as account {account}",
            start.program.display()
        ),
        _ => {
            format!("the unit for account {account} did not start ({result}, exit status {status})")
        }
    };
    account_start_error(ACCOUNT_UNAVAILABLE, detail)
}

#[cfg(test)]
#[path = "exec_account_tests.rs"]
pub(super) mod tests;
