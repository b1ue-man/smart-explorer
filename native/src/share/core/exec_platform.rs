use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Instant;

#[cfg(debug_assertions)]
use std::time::Duration;

use super::access_catalog::HelperStatus;
use super::exec_helper_protocol::{AccountChoice, AccountChoices};
use super::exec_supervisor_protocol::{
    environment_for, environment_for_account, RunAsAccount, SupervisorCommand, SupervisorEvent,
    SupervisorStart,
};
use super::exec_terminal::ExecTerminalSize;
#[cfg(debug_assertions)]
use super::exec_types::ExecId;
use super::exec_types::{ExecCommand, ExecProviderStatus, ExecStart};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReason {
    Cancelled,
    TimedOut,
    Revoked,
    Disconnected,
    WorkerStopping,
    RootExited,
    ProtocolError,
}

thread_local! {
    /// The account of the start that `ContainedExec::prepare_as` prepares on
    /// this thread; the platform's `prepare` reads it (`preparing_account`).
    static PREPARING_ACCOUNT: RefCell<Option<RunAsAccount>> = const { RefCell::new(None) };
}

pub(crate) struct ContainedExec {
    inner: super::platform_exec::ContainedExec,
    committed: bool,
    /// Name and uid of the account the platform prepared the start for;
    /// `None` for the worker's own account.
    prepared_for: Option<(String, u32)>,
}

impl ContainedExec {
    /// Prepares a start of the worker's own account.
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        Self::prepare_as(request, None)
    }

    /// Prepares a start that runs as `account` (`None`: the worker's own
    /// account). On Linux another account runs in its own unit, which has to
    /// exist before the start is committed (PRM-M5, PRM-M6); `commit`
    /// refuses any other account than this one.
    pub(crate) fn prepare_as(
        request: &ExecStart,
        account: Option<&RunAsAccount>,
    ) -> io::Result<Self> {
        if account.is_some() && !run_as_supported() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this host runs commands only as the worker's own account",
            ));
        }
        let slot = PreparingAccount::set(account);
        let inner = super::platform_exec::ContainedExec::prepare(request);
        drop(slot);
        Ok(Self {
            inner: inner?,
            committed: false,
            prepared_for: account.map(|account| (account.name.clone(), account.uid)),
        })
    }

    /// Releases the start. `account` is the grant's resolved run-as account;
    /// `None` runs the job as the worker's own account, as always before.
    /// `program` is the path the worker resolved and authorized for a grant
    /// with a program list: the supervisor then starts exactly that file and
    /// never looks the name up itself (not in the job's `PATH` either).
    pub(crate) fn commit(
        &mut self,
        mut request: ExecStart,
        account: Option<RunAsAccount>,
        program: Option<&Path>,
    ) -> io::Result<()> {
        request.validate()?;
        if self.committed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "execution already committed",
            ));
        }
        // `prepare_as` checked that this host may run the account at all.
        let requested = account
            .as_ref()
            .map(|account| (account.name.clone(), account.uid));
        if requested != self.prepared_for {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this start was prepared for another account than its grant names",
            ));
        }
        if let Some(path) = program {
            pin_program(&mut request, path)?;
        }
        let environment = match &account {
            Some(account) => environment_for_account(&request, account),
            None => environment_for(&request),
        };
        self.inner.configure(&request)?;
        self.inner.send(&SupervisorCommand::Start(SupervisorStart {
            request,
            environment,
            account,
        }))?;
        self.committed = true;
        Ok(())
    }

    /// Hands `bytes` to the supervisor in chunks of at most 64 KiB and returns
    /// how many bytes it took. A full supervisor input queue ends the hand-over
    /// early, possibly before the first chunk; the caller offers the rest again
    /// later, so no chunk is sent twice. Any other failure is an error.
    pub(crate) fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut written = 0;
        for chunk in bytes.chunks(super::exec_types::MAX_EXEC_DATA_BYTES) {
            match self.inner.send(&SupervisorCommand::Stdin(chunk.to_vec())) {
                Ok(()) => written += chunk.len(),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(written)
    }

    /// Ends the input. A full input queue is reported as `WouldBlock`.
    pub(crate) fn close_stdin(&mut self) -> io::Result<()> {
        self.inner.send(&SupervisorCommand::StdinEof)
    }

    /// Tells the supervisor the new window size of a terminal start. A full
    /// input queue is reported as `WouldBlock`.
    pub(crate) fn resize(&mut self, size: ExecTerminalSize) -> io::Result<()> {
        self.inner.send(&SupervisorCommand::Resize(size))
    }

    pub(crate) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        self.inner.next_event(deadline)
    }

    pub(crate) fn terminate_all(&mut self, reason: StopReason) -> io::Result<()> {
        self.inner.terminate_all(reason)
    }

    pub(crate) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        self.inner.confirm_empty(deadline)
    }
}

/// Replaces the program of an argv start by the authorized absolute path.
fn pin_program(request: &mut ExecStart, path: &Path) -> io::Result<()> {
    let ExecCommand::Argv { program, .. } = &mut request.command else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a resolved program path only applies to an argv start",
        ));
    };
    let path = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the resolved program path is not valid UTF-8",
        )
    })?;
    *program = path.to_string();
    Ok(())
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    super::platform_exec::provider_status()
}

/// Where this host looks bare program names up for a grant with a program
/// list: a fixed list of system directories, never the caller's `PATH`.
pub(crate) fn program_search() -> super::exec_policy::program_path::ProgramSearch {
    super::platform_exec::program_search()
}

/// True when this host can give an Exec start a terminal (PTY or ConPTY).
pub(crate) fn terminal_supported() -> bool {
    super::platform_exec::terminal_supported()
}

/// True when a grant may name another local account than the worker's.
pub(crate) fn run_as_supported() -> bool {
    super::platform_exec::run_as_supported()
}

/// Resolves a grant's run-as account on this host; `Ok(None)` for the
/// Standardnutzer (the worker's own account).
pub(crate) fn resolve_run_as(
    run_as: &super::exec_policy::ExecRunAs,
) -> io::Result<Option<RunAsAccount>> {
    match run_as.account_name() {
        None => Ok(None),
        Some(name) => super::platform_exec::resolve_account(name).map(Some),
    }
}

pub(super) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    super::platform_exec::run_supervisor_if_requested(arguments)
}

/// Sets `PREPARING_ACCOUNT` for one `prepare` and clears it afterwards, also
/// when the platform's `prepare` panics.
struct PreparingAccount;

impl PreparingAccount {
    fn set(account: Option<&RunAsAccount>) -> Self {
        PREPARING_ACCOUNT.with(|slot| *slot.borrow_mut() = account.cloned());
        Self
    }
}

impl Drop for PreparingAccount {
    fn drop(&mut self) {
        PREPARING_ACCOUNT.with(|slot| slot.borrow_mut().take());
    }
}

/// The account of the start `ContainedExec::prepare_as` prepares on this
/// thread; `None` for the worker's own account. Only the Linux platform
/// starts another account's unit in `prepare`; on the others `prepare_as`
/// refuses another account before.
#[allow(dead_code)]
pub(crate) fn preparing_account() -> Option<RunAsAccount> {
    PREPARING_ACCOUNT.with(|slot| slot.borrow().clone())
}

/// The `ExecWireError.code` of a start this host refused for its account
/// (`account_not_allowed`, `account_helper_unavailable`,
/// `account_unavailable`); `None` for every other error.
#[allow(dead_code)] // the Exec admission maps account refusals with it
pub(crate) fn account_error_code(error: &io::Error) -> Option<&'static str> {
    super::exec_helper_protocol::account_error_code(error)
}

/// The account choices of a host that runs commands only as the worker's
/// own account.
#[allow(dead_code)] // Windows and Android, until they have account services
pub(crate) fn own_account_choices(helper: HelperStatus) -> AccountChoices {
    AccountChoices {
        helper,
        choices: vec![AccountChoice::own(&provider_status().user_label)],
    }
}

#[cfg(debug_assertions)]
pub(super) fn run_platform_self_test() -> io::Result<()> {
    let status = provider_status();
    if !status.available {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{}: {}", status.provider, status.detail),
        ));
    }
    let request = ExecStart {
        exec_id: ExecId::generate()?,
        command: super::exec_types::ExecCommand::Shell {
            command: platform_test_command().into(),
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: Some(10_000),
        max_output_bytes: None,
        terminal: None,
    };
    let mut process = ContainedExec::prepare(&request)?;
    process.commit(request, None, None)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw_started = false;
    let mut saw_output = false;
    let mut saw_error_output = false;
    let mut saw_root_exit = false;
    loop {
        match process.next_event(Some(deadline))? {
            SupervisorEvent::Started { .. } => saw_started = true,
            SupervisorEvent::Stdout(bytes) => saw_output |= contains(&bytes, output_marker()),
            SupervisorEvent::Stderr(bytes) => saw_error_output |= contains(&bytes, error_marker()),
            SupervisorEvent::RootExited(_) => saw_root_exit = true,
            SupervisorEvent::Exited(exit) => {
                if !saw_root_exit || !saw_output || !saw_error_output {
                    return Err(io::Error::other(
                        "Exited overtook stdout/stderr or RootExited",
                    ));
                }
                if exit.code != Some(7) {
                    return Err(io::Error::other(format!(
                        "unexpected helper exit: {exit:?}"
                    )));
                }
                break;
            }
            SupervisorEvent::Error(message) => return Err(io::Error::other(message)),
        }
    }
    process.terminate_all(StopReason::RootExited)?;
    process.confirm_empty(Instant::now() + Duration::from_secs(10))?;
    if !saw_started {
        return Err(io::Error::other("platform self-test missed start"));
    }
    run_root_exits_first_test()?;
    super::platform_exec::run_extended_self_test()
}

#[cfg(debug_assertions)]
fn contains(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.windows(marker.len()).any(|window| window == marker)
}

#[cfg(debug_assertions)]
fn run_root_exits_first_test() -> io::Result<()> {
    let request = ExecStart {
        exec_id: ExecId::generate()?,
        command: super::exec_types::ExecCommand::Shell {
            command: root_exits_first_command().into(),
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: Some(10_000),
        max_output_bytes: None,
        terminal: None,
    };
    let mut process = ContainedExec::prepare(&request)?;
    process.commit(request, None, None)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match process.next_event(Some(deadline))? {
            SupervisorEvent::RootExited(exit) => {
                if exit.code != Some(9) {
                    return Err(io::Error::other(format!(
                        "unexpected root-exits-first status: {exit:?}"
                    )));
                }
                break;
            }
            SupervisorEvent::Error(message) => return Err(io::Error::other(message)),
            _ => {}
        }
    }
    process.terminate_all(StopReason::RootExited)?;
    process.confirm_empty(Instant::now() + Duration::from_secs(10))
}

#[cfg(all(debug_assertions, not(windows)))]
fn platform_test_command() -> &'static str {
    "printf 'SE-EXEC\\000'; printf 'SE-ERR\\000' >&2; exit 7"
}

#[cfg(all(debug_assertions, not(windows)))]
fn output_marker() -> &'static [u8] {
    b"SE-EXEC\0"
}

#[cfg(all(debug_assertions, not(windows)))]
fn error_marker() -> &'static [u8] {
    b"SE-ERR\0"
}

#[cfg(all(debug_assertions, not(windows)))]
fn root_exits_first_command() -> &'static str {
    "setsid sh -c 'trap \"\" TERM; while :; do sleep 1; done' & exit 9"
}

#[cfg(all(debug_assertions, windows))]
fn platform_test_command() -> &'static str {
    "<nul set /p =SE-EXEC & <nul set /p =SE-ERR 1>&2 & exit /b 7"
}

#[cfg(all(debug_assertions, windows))]
fn output_marker() -> &'static [u8] {
    b"SE-EXEC"
}

#[cfg(all(debug_assertions, windows))]
fn error_marker() -> &'static [u8] {
    b"SE-ERR"
}

#[cfg(all(debug_assertions, windows))]
fn root_exits_first_command() -> &'static str {
    "start \"\" /b cmd /D /S /C \"ping -t 127.0.0.1 >nul\" & exit /b 9"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn start(command: ExecCommand) -> ExecStart {
        ExecStart {
            exec_id: super::super::exec_types::ExecId::parse("0c".repeat(16)).unwrap(),
            command,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            max_output_bytes: None,
            terminal: None,
        }
    }

    #[test]
    fn luecken_task_the_supervisor_gets_the_authorized_path_instead_of_the_name() {
        let mut request = start(ExecCommand::Argv {
            program: "uptime".into(),
            args: vec!["-p".into()],
        });
        pin_program(&mut request, Path::new("/usr/bin/uptime")).unwrap();
        assert_eq!(
            request.command,
            ExecCommand::Argv {
                program: "/usr/bin/uptime".into(),
                args: vec!["-p".into()],
            }
        );
        let mut shell = start(ExecCommand::Shell {
            command: "true".into(),
        });
        assert!(pin_program(&mut shell, Path::new("/usr/bin/uptime")).is_err());
    }
}
