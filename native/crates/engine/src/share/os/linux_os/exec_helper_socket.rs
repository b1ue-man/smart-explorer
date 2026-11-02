//! Bringing the account service's socket unit into a fresh, listening state
//! (`exec_helper_setup.rs`). `RestartUnit` only queues its job, so the
//! socket file can still be missing when the call returns; and systemd
//! counts every start of the unit against its start limit (five within ten
//! seconds), after which the unit stays `failed` and its socket is gone
//! (`RemoveOnStop=yes`). Setting up or renewing the service repeatedly
//! therefore resets that counter, stops the unit, waits until it is
//! inactive, starts it and waits until it listens.
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use super::super::exec_helper_client::HELPER_SOCKET;
use super::super::exec_systemd;
use super::SOCKET_UNIT;

/// How long a stop or a start of the socket unit may take.
const SETTLE: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

/// Stops the socket unit (when it runs), starts it anew and returns when
/// it is active and `HELPER_SOCKET` is a socket.
pub(super) fn restart_listening(manager: &Connection) -> io::Result<()> {
    exec_systemd::reset_failed_unit(manager, SOCKET_UNIT)?;
    stop_inactive(manager)?;
    // The stop counted nothing, but a failed state left by it must not
    // block the start.
    exec_systemd::reset_failed_unit(manager, SOCKET_UNIT)?;
    exec_systemd::restart_unit(manager, SOCKET_UNIT)?;
    wait_for(manager, "start", |state| state == "active")?;
    let deadline = Instant::now() + SETTLE;
    while !is_socket(HELPER_SOCKET) {
        if Instant::now() >= deadline {
            return Err(timed_out(format!(
                "{SOCKET_UNIT} is active, but {HELPER_SOCKET} is no socket"
            )));
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

/// Stops the socket unit and returns when it is inactive, so that its
/// socket file (`RemoveOnStop=yes`) is gone.
pub(super) fn stop_inactive(manager: &Connection) -> io::Result<()> {
    exec_systemd::stop_unit(manager, SOCKET_UNIT)?;
    wait_for(manager, "stop", |state| {
        matches!(state, "" | "inactive" | "failed")
    })
}

/// Polls the unit's `ActiveState` (`""` while systemd has not loaded the
/// unit) until `reached` accepts it. A unit that is `failed` while a start
/// is awaited has failed for good: its state is reported at once.
fn wait_for(manager: &Connection, action: &str, reached: impl Fn(&str) -> bool) -> io::Result<()> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let state = match exec_systemd::loaded_unit(manager, SOCKET_UNIT)? {
            Some(path) => exec_systemd::unit_active_state(manager, &path)?,
            None => String::new(),
        };
        if reached(&state) {
            return Ok(());
        }
        if state == "failed" && action == "start" {
            return Err(io::Error::other(format!(
                "{SOCKET_UNIT} failed to start (systemctl status {SOCKET_UNIT})"
            )));
        }
        if Instant::now() >= deadline {
            return Err(timed_out(format!(
                "{SOCKET_UNIT} did not finish its {action} (state {state:?})"
            )));
        }
        std::thread::sleep(POLL);
    }
}

fn is_socket(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn timed_out(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message)
}
