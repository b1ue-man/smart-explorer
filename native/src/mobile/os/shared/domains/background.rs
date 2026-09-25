//! `bg.*`: the embedded desktop worker (one thread of the app process). The
//! controls are the desktop settings calls; "Aus" is the sync flag.
use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{json, Value};

use super::args::{bool_arg, canceled, i64_arg, invalid, io_error, now_ms, opt_i64, text_error};
use crate::mobile::{ApiError, Runtime, TaskCtx};

const READY_WAIT: Duration = Duration::from_secs(10);
const CATCH_UP_POLL: Duration = Duration::from_millis(500);
const DEFAULT_LOG_BYTES: i64 = 64 * 1024;
const MAX_LOG_BYTES: i64 = 1024 * 1024;
/// Lines read before trimming to the requested byte budget.
const LOG_LINES: usize = 20_000;

/// Catch-up runs started by this facade that have not finished yet.
static CATCH_UPS: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());

fn wait_ready() -> Result<(), ApiError> {
    match crate::daemon::ensure_embedded_daemon(READY_WAIT) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ApiError::new(
            "busy",
            "Background-Worker wurde nicht rechtzeitig bereit.",
        )),
        Err(error) => Err(ApiError::new("internal", error)),
    }
}

pub(super) fn ensure_daemon() -> Result<Value, ApiError> {
    let running = crate::daemon::ensure_embedded_daemon(READY_WAIT)
        .map_err(|error| ApiError::new("internal", error))?;
    Ok(json!({ "running": running }))
}

pub(super) fn status() -> Result<Value, ApiError> {
    let pause =
        crate::daemon::pause_remaining().map_err(|error| io_error("Pausenstatus lesen", error))?;
    let (battery, metered) = crate::daemon::autopause_flags()
        .map_err(|error| io_error("Automatische Pause lesen", error))?;
    let cadence = crate::daemon::cadence_secs().map_err(|error| io_error("Takt lesen", error))?;
    let paused_until = pause
        .filter(|seconds| *seconds != i64::MAX)
        .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1000)));
    let catch_up_running = !CATCH_UPS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    Ok(json!({
        "syncEnabled": crate::autostart::is_enabled(),
        "daemonRunning": crate::daemon::is_running(),
        "heartbeatAgeSecs": crate::daemon::last_heartbeat_age(),
        "paused": pause.is_some(),
        "pausedUntilMs": paused_until,
        "autopauseBattery": battery,
        "autopauseMetered": metered,
        "cadenceSecs": cadence,
        "catchUpRunning": catch_up_running,
        "lastCatchUpMs": crate::daemon::last_catch_up_ms(),
        "activeJob": crate::daemon::active_job(),
    }))
}

/// Desktop settings flow: enabling also makes sure the worker runs and rolls
/// back when that fails; disabling only stops scheduling (Share stays).
pub(super) fn set_sync_enabled(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    if bool_arg(args, "enabled")? {
        crate::autostart::enable()
            .map_err(|error| io_error("Hintergrund-Sync einschalten", error))?;
        if let Err(error) = crate::daemon::request_daemon_replacement() {
            let rollback = crate::autostart::disable()
                .err()
                .map(|rollback| format!("; Zurücksetzen fehlgeschlagen: {rollback}"))
                .unwrap_or_default();
            return Err(ApiError::new(
                "internal",
                format!("Background-Worker konnte nicht starten: {error}{rollback}"),
            ));
        }
    } else {
        crate::autostart::disable()
            .map_err(|error| io_error("Hintergrund-Sync ausschalten", error))?;
    }
    rt.emit(json!({ "type": "jobs" }));
    Ok(json!({}))
}

pub(super) fn pause(args: &Value) -> Result<Value, ApiError> {
    let seconds = i64_arg(args, "seconds")?;
    let result = match seconds {
        -1 => crate::daemon::pause_indefinite(),
        seconds if seconds > 0 => crate::daemon::pause_for_secs(seconds),
        _ => {
            return Err(invalid(
                "Die Pausendauer muss positiv sein (−1 = unbegrenzt).",
            ))
        }
    };
    result.map_err(|error| io_error("Pausieren", error))?;
    Ok(json!({}))
}

pub(super) fn resume() -> Result<Value, ApiError> {
    crate::daemon::resume().map_err(|error| io_error("Fortsetzen", error))?;
    Ok(json!({}))
}

pub(super) fn set_autopause(args: &Value) -> Result<Value, ApiError> {
    let battery = bool_arg(args, "battery")?;
    let metered = bool_arg(args, "metered")?;
    crate::daemon::set_autopause_flags(battery, metered)
        .map_err(|error| io_error("Automatische Pause speichern", error))?;
    Ok(json!({}))
}

pub(super) fn log(args: &Value) -> Result<Value, ApiError> {
    let budget = opt_i64(args, "maxBytes")
        .unwrap_or(DEFAULT_LOG_BYTES)
        .clamp(1, MAX_LOG_BYTES) as usize;
    let text = crate::daemon::read_log_tail(LOG_LINES);
    Ok(json!({ "text": tail_bytes(&text, budget) }))
}

/// The last `budget` bytes, starting at a line (or at least a char) boundary.
pub(super) fn tail_bytes(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut start = text.len() - budget;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let tail = &text[start..];
    match tail.find('\n') {
        Some(newline) if newline + 1 < tail.len() => &tail[newline + 1..],
        _ => tail,
    }
}

pub(super) fn catch_up(rt: &Runtime) -> Result<Value, ApiError> {
    let task = rt.spawn_task("catchup", "Hintergrund-Lauf".to_string(), catch_up_task);
    Ok(json!({ "taskId": task }))
}

fn catch_up_task(ctx: &TaskCtx) -> Result<Value, ApiError> {
    ctx.message("Starte Background-Worker…");
    wait_ready()?;
    let id = crate::daemon::request_catch_up()
        .map_err(|error| text_error("internal", "Nachhol-Lauf", error))?;
    CATCH_UPS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id);
    let outcome = follow_catch_up(ctx, id);
    CATCH_UPS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    if let Ok(rt) = Runtime::get() {
        rt.emit(json!({ "type": "jobs" }));
    }
    outcome
}

fn follow_catch_up(ctx: &TaskCtx, id: u64) -> Result<Value, ApiError> {
    let mut cancel_sent = false;
    loop {
        if ctx.cancelled() && !cancel_sent {
            crate::daemon::cancel_catch_up(id);
            cancel_sent = true;
        }
        let Some(status) = crate::daemon::catch_up_status(id) else {
            return Err(ApiError::new(
                "internal",
                "Status des Nachhol-Laufs ist nicht mehr verfügbar.",
            ));
        };
        let running = usize::from(status.running_job.is_some());
        let done = status
            .admitted
            .saturating_sub(status.queued)
            .saturating_sub(running);
        ctx.progress(0, 0, done as u64, status.admitted as u64);
        let text = status
            .running_job
            .as_ref()
            .map(|job| format!("Synchronisiere „{job}“…"))
            .or_else(|| status.message.clone());
        if let Some(text) = &text {
            ctx.message(text);
        }
        if status.finished {
            let message = status.message.clone().unwrap_or_default();
            if cancel_sent {
                return Err(canceled(if message.is_empty() {
                    "Abgebrochen".to_string()
                } else {
                    message
                }));
            }
            let skipped: Vec<Value> = status
                .skipped
                .iter()
                .map(|skip| {
                    json!({
                        "jobId": skip.job_id,
                        "jobName": skip.job_name,
                        "reason": skip.reason,
                    })
                })
                .collect();
            return Ok(json!({
                "admitted": status.admitted,
                "skipped": skipped,
                "message": status.message,
            }));
        }
        std::thread::sleep(CATCH_UP_POLL);
    }
}
