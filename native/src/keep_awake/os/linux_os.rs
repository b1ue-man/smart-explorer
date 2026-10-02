//! Linux backend of the wake holds (RV1, V4): one logind inhibitor lock per
//! reason over the system D-Bus (`org.freedesktop.login1.Manager.Inhibit`,
//! also offered by elogind). The lock is a file descriptor; closing it (or
//! the end of the process) releases it, so a crash never keeps the machine
//! awake. `idle:sleep` in mode `block` stops idle and key suspend; outside a
//! local session polkit may refuse `sleep`, then `idle` alone is taken. The
//! lid switch keeps working (logind default `LidSwitchIgnoreInhibited=yes`).

use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedFd;

use super::types::{Applied, Reason};

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";
const WHO: &str = "Smart Explorer";
/// After a failure the bus is asked again at most this often, so many short
/// holds on a system without logind do not hammer it.
const RETRY_AFTER: Duration = Duration::from_secs(60);

pub(super) struct Backend {
    connection: Option<Connection>,
    locks: [Option<OwnedFd>; Reason::ALL.len()],
    failed_at: Option<Instant>,
    last_error: Option<String>,
}

impl Backend {
    pub(super) fn new() -> Self {
        Self {
            connection: None,
            locks: [None, None, None],
            failed_at: None,
            last_error: None,
        }
    }

    pub(super) fn apply(&mut self, held: [bool; Reason::ALL.len()]) -> Applied {
        for reason in Reason::ALL {
            let index = reason.index();
            if !held[index] {
                // Dropping the descriptor releases the inhibitor.
                self.locks[index] = None;
            } else if self.locks[index].is_none() && self.may_try() {
                match self.inhibit(reason) {
                    Ok(lock) => {
                        self.locks[index] = Some(lock);
                        self.failed_at = None;
                        self.last_error = None;
                    }
                    Err(error) => {
                        self.failed_at = Some(Instant::now());
                        self.last_error = Some(error);
                        // A broken connection is rebuilt on the next try.
                        self.connection = None;
                    }
                }
            }
        }
        if self.locks.iter().all(Option::is_none) {
            self.connection = None;
        }
        Applied {
            engaged: self.locks.iter().any(Option::is_some),
            throttling_off: false,
            unavailable: self.last_error.clone(),
        }
    }

    pub(super) fn renew_after(&self) -> Option<Duration> {
        // A hold that failed is tried again once the pause is over.
        self.failed_at.map(|_| RETRY_AFTER)
    }

    pub(super) fn wake_receiver(&self) -> Option<Receiver<()>> {
        None
    }

    fn may_try(&self) -> bool {
        self.failed_at
            .is_none_or(|failed| failed.elapsed() >= RETRY_AFTER)
    }

    fn inhibit(&mut self, reason: Reason) -> Result<OwnedFd, String> {
        if self.connection.is_none() {
            let connection = Connection::system()
                .map_err(|error| format!("Wachhalten: kein System-D-Bus ({error})"))?;
            self.connection = Some(connection);
        }
        let Some(connection) = &self.connection else {
            return Err("Wachhalten: kein System-D-Bus".into());
        };
        inhibit(connection, "idle:sleep", reason)
            .or_else(|_| inhibit(connection, "idle", reason))
            .map_err(|error| format!("Wachhalten: logind lehnt ab ({error})"))
    }
}

fn inhibit(connection: &Connection, what: &str, reason: Reason) -> zbus::Result<OwnedFd> {
    let reply = connection.call_method(
        Some(LOGIN1),
        LOGIN1_PATH,
        Some(LOGIN1_MANAGER),
        "Inhibit",
        &(what, WHO, reason.label(), "block"),
    )?;
    reply.body().deserialize::<OwnedFd>()
}
