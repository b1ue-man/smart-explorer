//! The session's shell on this device (the MCP `shell` tool with `session`,
//! and the tests of the session engine): a child process with piped streams
//! started through `engine::elevate`, directly or elevated (one system
//! dialog for the whole session), in its own process group or job object.
//! Cancel ends the shell and every process it started.
use super::transports::{Transport, TransportEnd, TransportEvent, TransportInput};
use crate::elevate::{ElevatedChild, ElevatedEvent, ElevatedInput, ElevatedSpec};
use std::sync::Arc;

pub(super) struct LocalTransport {
    child: ElevatedChild,
    input: Arc<LocalInput>,
}

struct LocalInput(ElevatedInput);

impl TransportInput for LocalInput {
    fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.stdin(bytes)
    }

    fn eof(&self) -> Result<(), String> {
        self.0.eof()
    }

    fn cancel(&self) {
        self.0.cancel();
    }
}

impl LocalTransport {
    /// Starts `start` with se's rights or, when `elevated`, with
    /// administrator or root rights (blocks until the user answered).
    pub(super) fn start(start: ElevatedSpec, elevated: bool) -> Result<Self, String> {
        let child = if elevated {
            crate::elevate::spawn_elevated(start)
        } else {
            crate::elevate::spawn_direct(start)
        }
        .map_err(|error| error.to_string())?;
        let input = Arc::new(LocalInput(child.input()));
        Ok(Self { child, input })
    }

    /// The tests' shell: `program args` with se's rights.
    #[cfg(test)]
    pub(super) fn spawn(program: &str, args: &[&str]) -> Self {
        let start = ElevatedSpec {
            command: crate::elevate::ElevatedCommand::Argv {
                program: program.to_string(),
                args: args.iter().map(|arg| arg.to_string()).collect(),
            },
            cwd: None,
            env: Default::default(),
        };
        Self::start(start, false).expect("start the local test shell")
    }
}

impl Transport for LocalTransport {
    fn next_event(&mut self) -> TransportEvent {
        match self.child.next_event() {
            Some(ElevatedEvent::Stdout(bytes)) => TransportEvent::Stdout(bytes),
            Some(ElevatedEvent::Stderr(bytes)) => TransportEvent::Stderr(bytes),
            Some(ElevatedEvent::Exited(exit)) => TransportEvent::Ended(TransportEnd {
                exit_code: if exit.cancelled { None } else { exit.status() },
                cancelled: exit.cancelled,
                error: exit.error,
            }),
            None => TransportEvent::Ended(TransportEnd {
                error: Some("the local shell is gone".into()),
                ..TransportEnd::default()
            }),
        }
    }

    fn input(&self) -> Arc<dyn TransportInput> {
        self.input.clone()
    }
}
