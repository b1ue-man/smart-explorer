//! Android has no contained remote-execution provider: an app process can
//! neither create cgroup-backed services nor supervise detached child process
//! trees. The provider reports itself unavailable and every preparation fails
//! with `Unsupported`, so a remote execution request is refused before anything
//! starts. `ContainedExec` is uninhabited: no value can exist on Android.
use std::convert::Infallible;
use std::ffi::OsString;
use std::io;
use std::time::Instant;

use crate::share::exec_platform::StopReason;
use crate::share::exec_supervisor_protocol::{SupervisorCommand, SupervisorEvent};
use crate::share::exec_types::{ExecProviderStatus, ExecStart};

const PROVIDER: &str = "Android";
const UNAVAILABLE: &str = "Entfernte Ausführung ist unter Android nicht verfügbar";

pub(crate) struct ContainedExec {
    never: Infallible,
}

impl ContainedExec {
    pub(crate) fn prepare(_request: &ExecStart) -> io::Result<Self> {
        Err(unsupported())
    }

    pub(crate) fn configure(&mut self, _request: &ExecStart) -> io::Result<()> {
        match self.never {}
    }

    pub(crate) fn send(&mut self, _command: &SupervisorCommand) -> io::Result<()> {
        match self.never {}
    }

    pub(crate) fn next_event(&mut self, _deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        match self.never {}
    }

    pub(crate) fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        match self.never {}
    }

    pub(crate) fn confirm_empty(&mut self, _deadline: Instant) -> io::Result<()> {
        match self.never {}
    }
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    ExecProviderStatus {
        available: false,
        provider: PROVIDER.into(),
        detail: UNAVAILABLE.into(),
        elevated: false,
        user_label: "Android-App".into(),
    }
}

/// Android never runs as the hidden supervisor process.
pub(crate) fn run_supervisor_if_requested(_arguments: &[OsString]) -> Option<io::Result<()>> {
    None
}

#[cfg(debug_assertions)]
pub(crate) fn run_extended_self_test() -> io::Result<()> {
    Err(unsupported())
}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, UNAVAILABLE)
}
