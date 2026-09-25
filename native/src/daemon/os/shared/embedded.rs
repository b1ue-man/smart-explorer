//! The embedded background worker: the same loop as the desktop
//! `--sync-daemon` process, hosted as one thread of the app process (Android).
//! It starts on first need and is never stopped because of the UI; the host
//! process lifetime replaces the process-level stop and handoff. It uses no
//! environment handoff: every start is a fresh generation.

use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::handoff::stop_requested_for;
use super::live;
use super::run_loop::run_daemon_with;
use super::state::log;

const READY_POLL: Duration = Duration::from_millis(50);

static WORKER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// Start the worker thread unless it runs, then wait up to `timeout` for it to
/// serve. `Ok(false)` = still starting; `Err` = the thread could not be
/// started or ended without becoming ready (details in the worker log).
pub fn ensure_embedded_daemon(timeout: Duration) -> Result<bool, String> {
    start_if_needed()?;
    let deadline = Instant::now().checked_add(timeout);
    loop {
        if ready() {
            return Ok(true);
        }
        if !worker_alive() {
            return Err("Background-Worker konnte nicht starten (siehe Worker-Protokoll)".into());
        }
        let now = Instant::now();
        let wait = match deadline {
            Some(deadline) if now >= deadline => return Ok(false),
            Some(deadline) => READY_POLL.min(deadline - now),
            None => READY_POLL,
        };
        std::thread::sleep(wait);
    }
}

/// Client-side readiness (Share IPC helpers): the thread replaces launching
/// or handing off to a worker process.
pub(super) fn ensure_for_client(timeout: Duration) -> Result<(), String> {
    if ensure_embedded_daemon(timeout)? {
        Ok(())
    } else {
        Err("Background-Worker wurde nicht rechtzeitig bereit".into())
    }
}

fn start_if_needed() -> Result<(), String> {
    let mut worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return Ok(());
    }
    if let Some(finished) = worker.take() {
        if finished.join().is_err() {
            log("embedded background worker panicked; starting it again");
        }
    }
    live::mark_embedded();
    let handle = std::thread::Builder::new()
        .name("background-worker".into())
        .spawn(|| run_daemon_with(None))
        .map_err(|error| {
            let message = format!("Background-Worker starten: {error}");
            log(&message);
            message
        })?;
    *worker = Some(handle);
    Ok(())
}

fn worker_alive() -> bool {
    WORKER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|handle| !handle.is_finished())
}

/// Initialized and not asked to stop - the in-process equivalent of a ready
/// Ping from the current generation.
fn ready() -> bool {
    live::serving_generation().is_some_and(|generation| !stop_requested_for(&generation))
}
