//! In-memory Exec process double for unit and measurement tests (`--lib`),
//! where `std::env::current_exe()` is the test binary rather than the
//! supervisor binary and no external `systemd --user` service is assumed.
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::exec_platform::{ContainedExec, StopReason};
use super::super::exec_supervisor_protocol::{RunAsAccount, SupervisorEvent, SupervisorExit};
use super::super::exec_terminal::ExecTerminalSize;
use super::super::exec_types::{ExecProviderStatus, ExecStart};

/// How one simulated Exec process behaves after `commit`.
pub(in crate::share) enum TestExecPlan {
    /// Emits `Started`, optional output chunks, and `Exited(exit_code)`.
    Immediate {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    /// Emits `Started`, marks `started`, and stays alive until stopped or
    /// cancelled so tests can exercise mid-job revocation or cancellation.
    HoldUntilStopped { started: Arc<AtomicBool> },
}

type PlanFactory = dyn Fn(&ExecStart) -> io::Result<TestExecPlan> + Send + Sync;

/// Per-node test hook replacing only the OS supervisor spawn while keeping
/// admission, grants, registry, heartbeats, presence and audit intact.
#[derive(Clone)]
pub(in crate::share) struct TestExecRunner {
    provider: ExecProviderStatus,
    terminal_supported: bool,
    factory: Arc<PlanFactory>,
}

impl TestExecRunner {
    pub(in crate::share) fn new(
        factory: impl Fn(&ExecStart) -> io::Result<TestExecPlan> + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider: ExecProviderStatus {
                available: true,
                provider: "test-runner".into(),
                detail: "in-memory test runner".into(),
                user_label: "testuser".into(),
                elevated: false,
            },
            terminal_supported: true,
            factory: Arc::new(factory),
        }
    }

    pub(in crate::share) fn immediate_ok() -> Self {
        Self::new(|_| {
            Ok(TestExecPlan::Immediate {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: 0,
            })
        })
    }

    pub(in crate::share) fn provider(&self) -> (ExecProviderStatus, bool) {
        (self.provider.clone(), self.terminal_supported)
    }

    fn prepare(&self, request: &ExecStart) -> io::Result<TestJobProcess> {
        let plan = (self.factory)(request)?;
        Ok(TestJobProcess {
            plan: Some(plan),
            events: VecDeque::new(),
            hold_flag: None,
            committed: false,
            stopped: false,
        })
    }
}

pub(super) enum JobProcess {
    Platform(ContainedExec),
    Test(TestJobProcess),
}

impl JobProcess {
    pub(super) fn prepare(
        request: &ExecStart,
        account: Option<&RunAsAccount>,
        runner: Option<&TestExecRunner>,
    ) -> io::Result<Self> {
        match runner {
            Some(runner) => runner.prepare(request).map(Self::Test),
            None => ContainedExec::prepare_as(request, account).map(Self::Platform),
        }
    }

    pub(super) fn commit(
        &mut self,
        request: ExecStart,
        account: Option<RunAsAccount>,
        program: Option<&Path>,
    ) -> io::Result<()> {
        match self {
            Self::Platform(inner) => inner.commit(request, account, program),
            Self::Test(inner) => inner.commit(request, account, program),
        }
    }

    pub(super) fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Platform(inner) => inner.write_stdin(bytes),
            Self::Test(inner) => inner.write_stdin(bytes),
        }
    }

    pub(super) fn close_stdin(&mut self) -> io::Result<()> {
        match self {
            Self::Platform(inner) => inner.close_stdin(),
            Self::Test(inner) => inner.close_stdin(),
        }
    }

    pub(super) fn resize(&mut self, size: ExecTerminalSize) -> io::Result<()> {
        match self {
            Self::Platform(inner) => inner.resize(size),
            Self::Test(inner) => inner.resize(size),
        }
    }

    pub(super) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        match self {
            Self::Platform(inner) => inner.next_event(deadline),
            Self::Test(inner) => inner.next_event(deadline),
        }
    }

    pub(super) fn terminate_all(&mut self, reason: StopReason) -> io::Result<()> {
        match self {
            Self::Platform(inner) => inner.terminate_all(reason),
            Self::Test(inner) => inner.terminate_all(reason),
        }
    }

    pub(super) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        match self {
            Self::Platform(inner) => inner.confirm_empty(deadline),
            Self::Test(inner) => inner.confirm_empty(deadline),
        }
    }

    /// A test plan runs no shell, so its result names none.
    pub(super) fn shell_label(&self) -> Option<String> {
        match self {
            Self::Platform(inner) => inner.shell_label(),
            Self::Test(_) => None,
        }
    }
}

pub(super) struct TestJobProcess {
    plan: Option<TestExecPlan>,
    events: VecDeque<SupervisorEvent>,
    hold_flag: Option<Arc<AtomicBool>>,
    committed: bool,
    stopped: bool,
}

impl TestJobProcess {
    fn commit(
        &mut self,
        request: ExecStart,
        _account: Option<RunAsAccount>,
        _program: Option<&Path>,
    ) -> io::Result<()> {
        request.validate()?;
        if self.committed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "execution already committed",
            ));
        }
        self.committed = true;
        self.events.push_back(SupervisorEvent::Started { pid: 1 });
        match self.plan.take() {
            Some(TestExecPlan::Immediate {
                stdout,
                stderr,
                exit_code,
            }) => {
                if !stdout.is_empty() {
                    self.events.push_back(SupervisorEvent::Stdout(stdout));
                }
                if !stderr.is_empty() {
                    self.events.push_back(SupervisorEvent::Stderr(stderr));
                }
                self.events
                    .push_back(SupervisorEvent::Exited(SupervisorExit {
                        code: Some(exit_code),
                        signal: None,
                        output_truncated: false,
                    }));
            }
            Some(TestExecPlan::HoldUntilStopped { started }) => {
                self.hold_flag = Some(started);
            }
            None => {}
        }
        Ok(())
    }

    fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn close_stdin(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn resize(&mut self, _size: ExecTerminalSize) -> io::Result<()> {
        Ok(())
    }

    fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        if let Some(event) = self.events.pop_front() {
            if matches!(event, SupervisorEvent::Started { .. }) {
                if let Some(flag) = &self.hold_flag {
                    flag.store(true, Ordering::Release);
                }
            }
            return Ok(event);
        }
        if !self.stopped {
            if let Some(until) = deadline {
                let now = Instant::now();
                if until > now {
                    std::thread::sleep((until - now).min(Duration::from_millis(10)));
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"))
    }

    fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        self.stopped = true;
        Ok(())
    }

    fn confirm_empty(&mut self, _deadline: Instant) -> io::Result<()> {
        Ok(())
    }
}
