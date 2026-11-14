//! The daemon side of the Linux screen host for a worker without session
//! variables (E23, S3): its session kind, its desktop and what it learned
//! about a session follow the active session of `seat0` at every request
//! (logind is read at most once per second, `active_session::current`), so
//! a switch to a greeter, another account or a lock screen on another
//! display changes monitors, windows, limits and input with it. A worker
//! with its own session variables keeps the kind it found at its start and
//! its own `XDG_CURRENT_DESKTOP`, as before.
use std::sync::{Mutex, PoisonError};

use super::SessionKind;

/// Whether a backend follows the active session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Follow {
    follows: bool,
}

impl Follow {
    /// This process: it follows when it has no session variables.
    pub fn of_process() -> Self {
        Self {
            follows: !super::spawn::has_own_session(&|name| std::env::var(name).ok()),
        }
    }

    /// A backend with a fixed kind (tests, processes with own variables).
    pub const fn fixed() -> Self {
        Self { follows: false }
    }

    pub fn follows(&self) -> bool {
        self.follows
    }

    /// The kind of session now: `stored` for a fixed backend, else the
    /// kind of the followed session from `followed`.
    pub fn session_with(
        &self,
        stored: SessionKind,
        followed: impl FnOnce() -> SessionKind,
    ) -> SessionKind {
        if self.follows {
            followed()
        } else {
            stored
        }
    }

    /// The kind of session now (see `session_with`).
    pub fn session(&self, stored: SessionKind) -> SessionKind {
        self.session_with(stored, || {
            super::current().map_or(SessionKind::None, |session| session.kind)
        })
    }

    /// `XDG_CURRENT_DESKTOP` of this process, or of the followed session.
    pub fn desktop(&self) -> String {
        if !self.follows {
            return std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        }
        super::current()
            .ok()
            .and_then(|session| {
                session
                    .env
                    .iter()
                    .find(|(name, _)| name == "XDG_CURRENT_DESKTOP")
                    .map(|(_, value)| value.clone())
            })
            .unwrap_or_default()
    }

    /// The session facts belong to: `None` for a fixed backend (one value
    /// for the life of the process), else the followed session's
    /// generation, or `Some("")` without one.
    pub fn generation(&self) -> Option<String> {
        if !self.follows {
            return None;
        }
        Some(
            super::current()
                .map(|session| session.generation().to_string())
                .unwrap_or_default(),
        )
    }
}

/// A value learned once per session (once per process for a fixed backend).
pub struct PerSession<T> {
    slot: Mutex<Option<(Option<String>, T)>>,
}

impl<T> Default for PerSession<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

impl<T: Clone> PerSession<T> {
    /// The value for `generation`, made with `make` when there is none yet
    /// or it belongs to another session.
    pub fn get(&self, generation: Option<String>, make: impl FnOnce() -> T) -> T {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((known, value)) = slot.as_ref() {
            if *known == generation {
                return value.clone();
            }
        }
        let value = make();
        *slot = Some((generation, value.clone()));
        value
    }
}
