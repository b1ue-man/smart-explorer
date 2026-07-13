use std::ffi::OsString;
use std::io;
use std::time::Instant;

#[cfg(debug_assertions)]
use std::time::Duration;

use super::exec_supervisor_protocol::{
    environment_for, SupervisorCommand, SupervisorEvent, SupervisorStart,
};
#[cfg(debug_assertions)]
use super::exec_types::ExecId;
use super::exec_types::{ExecProviderStatus, ExecStart};

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

pub(crate) struct ContainedExec {
    inner: super::platform_exec::ContainedExec,
    committed: bool,
}

impl ContainedExec {
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        Ok(Self {
            inner: super::platform_exec::ContainedExec::prepare(request)?,
            committed: false,
        })
    }

    pub(crate) fn commit(&mut self, request: ExecStart) -> io::Result<()> {
        request.validate()?;
        if self.committed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "execution already committed",
            ));
        }
        let environment = environment_for(&request);
        self.inner.configure(&request)?;
        self.inner.send(&SupervisorCommand::Start(SupervisorStart {
            request,
            environment,
        }))?;
        self.committed = true;
        Ok(())
    }

    pub(crate) fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(super::exec_types::MAX_EXEC_DATA_BYTES) {
            self.inner.send(&SupervisorCommand::Stdin(chunk.to_vec()))?;
        }
        Ok(())
    }

    pub(crate) fn close_stdin(&mut self) -> io::Result<()> {
        self.inner.send(&SupervisorCommand::StdinEof)
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

pub(crate) fn provider_status() -> ExecProviderStatus {
    super::platform_exec::provider_status()
}

pub(super) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    super::platform_exec::run_supervisor_if_requested(arguments)
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
    };
    let mut process = ContainedExec::prepare(&request)?;
    process.commit(request)?;
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
    Ok(())
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
    };
    let mut process = ContainedExec::prepare(&request)?;
    process.commit(request)?;
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

#[cfg(all(debug_assertions, target_os = "linux"))]
fn platform_test_command() -> &'static str {
    "printf 'SE-EXEC\\000'; printf 'SE-ERR\\000' >&2; exit 7"
}

#[cfg(all(debug_assertions, target_os = "linux"))]
fn output_marker() -> &'static [u8] {
    b"SE-EXEC\0"
}

#[cfg(all(debug_assertions, target_os = "linux"))]
fn error_marker() -> &'static [u8] {
    b"SE-ERR\0"
}

#[cfg(all(debug_assertions, target_os = "linux"))]
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
