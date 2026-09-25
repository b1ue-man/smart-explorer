//! Process-local view of the worker loop running in this process: its
//! generation once initialized, the job it is running, catch-up runs, and -
//! for the embedded worker only - its Share host for in-process event
//! draining. A desktop GUI or CLI process has no worker loop, so these calls
//! report "not running" there; they never reach another process.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::catch_up::{CatchUpBook, CatchUpGate, CatchUpStatus, ServiceReport};
use super::ipc::{ShareHost, ShareWorkerSnapshot};
use super::ipc_host::stop_service_locked;
use super::job_supervisor::JobSupervisor;
use super::state::{log, now_secs, read_optional, write_control};

const LAST_CATCH_UP_FILE: &str = "catchup.last";

struct Live {
    /// Set once this process hosts the worker as a thread (never on desktop).
    embedded: bool,
    /// Generation of the initialized loop serving in this process.
    generation: Option<String>,
    /// Share host of the embedded worker, for in-process draining.
    host: Option<ShareHost>,
    active_job: Option<String>,
    catch_up: CatchUpBook,
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    embedded: false,
    generation: None,
    host: None,
    active_job: None,
    catch_up: CatchUpBook::new(),
});

fn lock() -> MutexGuard<'static, Live> {
    // Every update leaves the state consistent, so a panic elsewhere must not
    // take the worker status down with it.
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ask the worker loop in this process for one catch-up run.
pub fn request_catch_up() -> Result<u64, String> {
    let mut live = lock();
    if live.generation.is_none() {
        return Err("Background-Worker läuft nicht".into());
    }
    live.catch_up.request()
}

/// Status of a catch-up run (`None` = unknown or long finished).
pub fn catch_up_status(id: u64) -> Option<CatchUpStatus> {
    lock().catch_up.status(id)
}

/// Cancel the running and queued jobs of this run only.
pub fn cancel_catch_up(id: u64) {
    let report = lock().catch_up.cancel(id);
    record_finished(&report);
}

/// Display name of the job the worker in this process is running.
pub fn active_job() -> Option<String> {
    lock().active_job.clone()
}

/// Unix milliseconds when the last catch-up run finished (`None` = never or
/// unreadable).
pub fn last_catch_up_ms() -> Option<i64> {
    let path = crate::support_dirs::sync_data_dir().join(LAST_CATCH_UP_FILE);
    read_optional(&path).ok()??.trim().parse().ok()
}

/// Drain Share events at the embedded worker's host without IPC. `None` when
/// this process does not embed the worker.
pub fn drain_share_events_in_process() -> Option<Result<ShareWorkerSnapshot, String>> {
    let host = {
        let live = lock();
        if !live.embedded {
            return None;
        }
        live.host.clone()
    };
    Some(match host {
        Some(host) => Ok(host.drain_for_ui()),
        None => Err("Background-Worker ist nicht bereit".into()),
    })
}

pub(super) fn mark_embedded() {
    lock().embedded = true;
}

pub(super) fn is_embedded() -> bool {
    lock().embedded
}

pub(super) fn serving_generation() -> Option<String> {
    lock().generation.clone()
}

/// Publish the initialized loop; dropping the guard withdraws it again.
pub(super) fn serve(host: &ShareHost) -> Serving {
    let mut live = lock();
    live.generation = Some(host.generation().to_string());
    let embedded_host = live.embedded.then(|| host.clone());
    live.host = embedded_host.clone();
    Serving {
        host: embedded_host,
    }
}

pub(super) struct Serving {
    host: Option<ShareHost>,
}

impl Drop for Serving {
    fn drop(&mut self) {
        let report = {
            let mut live = lock();
            live.generation = None;
            live.host = None;
            live.active_job = None;
            live.catch_up.finish_all("Background-Worker wurde beendet")
        };
        record_finished(&report);
        // The desktop worker's process ends here. The app process lives on,
        // so the embedded worker releases its network presence itself; a
        // later start then owns the only Share service.
        if let Some(host) = self.host.take() {
            host.shutdown_lan();
            let mut state = host.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(error) = stop_service_locked(&mut state) {
                log(&format!("embedded share worker stop failed: {error}"));
            }
        }
    }
}

/// Loop hook: account finished supervisor work, publish the active job and
/// advance catch-up runs. `gate` is evaluated only while a run is open.
pub(super) fn service_catch_up(supervisor: &mut JobSupervisor, gate: impl FnOnce() -> CatchUpGate) {
    let completed = supervisor.take_completed();
    let (open, requested) = {
        let mut live = lock();
        live.catch_up.observe_completed(&completed);
        live.active_job = supervisor.active_job_name().map(str::to_string);
        (
            live.catch_up.has_open_runs(),
            live.catch_up.has_requested_runs(),
        )
    };
    if !open {
        return;
    }
    let gate = gate();
    let jobs = (requested && matches!(gate, CatchUpGate::Open))
        .then(|| crate::syncjobs::load().map_err(|error| error.to_string()));
    let report = {
        let mut live = lock();
        let report = live
            .catch_up
            .service(supervisor, &gate, jobs.as_ref(), now_secs());
        live.active_job = supervisor.active_job_name().map(str::to_string);
        report
    };
    for (id, admitted, skipped) in &report.started {
        log(&format!(
            "catch-up run {id} started: {admitted} admitted, {skipped} skipped"
        ));
    }
    record_finished(&report);
}

fn record_finished(report: &ServiceReport) {
    if report.finished.is_empty() {
        return;
    }
    for (id, message) in &report.finished {
        log(&format!("catch-up run {id} finished: {message}"));
    }
    let path = crate::support_dirs::sync_data_dir().join(LAST_CATCH_UP_FILE);
    let now_ms = now_secs().saturating_mul(1000);
    if let Err(error) = write_control(&path, &now_ms.to_string()) {
        log(&format!(
            "catch-up finish time could not be stored: {error}"
        ));
    }
}
