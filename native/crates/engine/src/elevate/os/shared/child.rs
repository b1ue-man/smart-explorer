//! A command started by this module, elevated or with the caller's rights:
//! its events in order (`Exited` last) and its input, which other threads
//! may use while one thread reads the events. Dropping a child that has not
//! ended cancels it, so no elevated process outlives its owner.
use std::io;
use std::sync::mpsc;
use std::sync::Arc;

use super::types::{ElevatedEvent, ElevatedExit, Elevation};

/// Input side of a running command.
pub(super) trait InputPort: Send + Sync {
    fn stdin(&self, bytes: &[u8]) -> io::Result<()>;
    fn eof(&self) -> io::Result<()>;
    /// Ends the command's process tree; its `Exited` says `cancelled`.
    fn cancel(&self);
}

/// Standard input and cancellation of a started command; cheap to clone.
#[derive(Clone)]
pub struct ElevatedInput(Arc<dyn InputPort>);

impl ElevatedInput {
    pub fn stdin(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.stdin(bytes).map_err(|error| error.to_string())
    }

    /// Closes standard input.
    pub fn eof(&self) -> Result<(), String> {
        self.0.eof().map_err(|error| error.to_string())
    }

    /// Ends the command and every process it started.
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

pub struct ElevatedChild {
    events: mpsc::Receiver<ElevatedEvent>,
    input: ElevatedInput,
    via: Elevation,
    ended: bool,
}

impl ElevatedChild {
    pub(super) fn new(
        events: mpsc::Receiver<ElevatedEvent>,
        input: Arc<dyn InputPort>,
        via: Elevation,
    ) -> Self {
        Self {
            events,
            input: ElevatedInput(input),
            via,
            ended: false,
        }
    }

    pub fn input(&self) -> ElevatedInput {
        self.input.clone()
    }

    /// How the command got its rights.
    pub fn via(&self) -> Elevation {
        self.via
    }

    /// The next event; blocks. `None` once `Exited` was returned.
    pub fn next_event(&mut self) -> Option<ElevatedEvent> {
        if self.ended {
            return None;
        }
        let event = self.events.recv().unwrap_or_else(|_| lost());
        self.ended = matches!(event, ElevatedEvent::Exited(_));
        Some(event)
    }
}

fn lost() -> ElevatedEvent {
    ElevatedEvent::Exited(ElevatedExit {
        error: Some("the command's event channel closed without an end".into()),
        ..ElevatedExit::default()
    })
}

impl Drop for ElevatedChild {
    fn drop(&mut self) {
        if !self.ended {
            self.input.cancel();
        }
    }
}
