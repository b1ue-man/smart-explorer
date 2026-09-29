//! Workers: take the next folder or file, wait for its parent folder without
//! any permit, reserve its buffers, then take the permits and run it (K2).
//! A transient failure is retried once while nothing was published; a peer
//! that is too busy is waited for while the job still moves (K13); a
//! refused local read asks the access gate outside every permit; a target
//! that takes nothing more, or a run of connection failures, ends the job.
use super::super::access::AccessAnswer;
use super::super::engine_names::parent_rel;
use super::super::engine_policy::{
    connection_failure, ends_job_at_target, is_back_pressure, is_transient, overload_keeps_waiting,
    retry_delay,
};
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::folders::FolderError;
use super::ops::{self, At, Carry, Meter, OpError, Outcome, COPY_BUFFER};
use super::queue::{FileWork, Work, WorkerSlot};
use super::Engine;
use std::io;
use std::time::{Duration, Instant};

pub(super) fn run(engine: &Engine<'_>, _slot: WorkerSlot<'_>) {
    let mut buffer = vec![0u8; COPY_BUFFER];
    while let Some(work) = engine.queue.pop(engine.stop_flag(), super::WORKER_LINGER) {
        match work {
            Work::Dir { rel, source } => match engine.folders.ensure(&rel, engine.stop_flag()) {
                // A move removes an emptied source folder only once its
                // counterpart exists at the target.
                Ok(_) if engine.is_move() => super::lock(&engine.moved_dirs).push((source, rel)),
                Ok(_) => {}
                Err(error) => folder_failed(engine, &engine.folders.path_of(&rel), error),
            },
            Work::File(file) => match super::batch::members(engine, &file) {
                Some(members) => super::batch::run(engine, file, members, &mut buffer),
                None => run_file(engine, file, &mut buffer),
            },
        }
    }
}

/// A folder that could not be created, reported at `path`: a target that
/// takes nothing more ends the job, anything else is this folder's (and its
/// files') problem; a connection failure counts once for the breaker.
pub(super) fn folder_failed(engine: &Engine<'_>, path: &str, error: FolderError) {
    if error.kind == io::ErrorKind::Interrupted || engine.stopped() {
        return;
    }
    if ends_job_at_target(error.kind) {
        engine.fatal(error.message);
        return;
    }
    engine.issue(path, &error.message);
    if error.connection {
        breaker(engine, &error.message);
    }
}

/// The parent folder of `file`, created once; `None` when it failed (the
/// file is reported) or the job stops.
pub(super) fn parent_ready(engine: &Engine<'_>, file: &FileWork) -> Option<bool> {
    let Some(parent) = parent_rel(&file.rel) else {
        return Some(false);
    };
    match engine.folders.ensure(parent, engine.stop_flag()) {
        Ok(created) => Some(created),
        Err(error) => {
            folder_failed(engine, &file.source, error);
            None
        }
    }
}

pub(super) fn run_file(engine: &Engine<'_>, mut file: FileWork, buffer: &mut [u8]) {
    let Some(parent_created) = parent_ready(engine, &file) else {
        return;
    };
    let mut carry = Carry::default();
    let mut asked_access = false;
    loop {
        let Some(reservation) = reserve_memory(ops::reservation(engine), engine.stop_flag()) else {
            return;
        };
        let Some(permits) = engine.acquire() else {
            return;
        };
        let active = engine.stats.begin(&file.rel);
        engine.queue.notify();
        let meter = Meter::new(&permits, &engine.stats);
        let result = ops::transfer(engine, &file, parent_created, &mut carry, &meter, buffer);
        let moved = meter.moved();
        permits.finish(match &result {
            Ok(_) => OpOutcome::Done,
            Err(failure) => classify_error(&failure.error),
        });
        drop(active);
        drop(reservation);
        let failure = match result {
            Ok(outcome) => {
                finished(engine, outcome);
                return;
            }
            Err(failure) => failure,
        };
        engine.stats.unmoved(moved);
        match decide(engine, &mut file, &failure, asked_access) {
            Decision::Stop => return,
            Decision::Retry(delay) => {
                file.retried = true;
                if !super::sleep_unless(engine.stop_flag(), delay) {
                    return;
                }
            }
            Decision::Wait(delay) => {
                if !super::sleep_unless(engine.stop_flag(), delay) {
                    return;
                }
            }
            Decision::Access => {
                asked_access = true;
                if !ask_access(engine, &file, &failure) {
                    return;
                }
            }
            Decision::Fatal(message) => {
                engine.fatal(message);
                return;
            }
            Decision::Report => {
                failed(engine, &file.source, &failure);
                return;
            }
        }
    }
}

pub(super) fn finished(engine: &Engine<'_>, outcome: Outcome) {
    match outcome {
        Outcome::Done => engine.stats.file_done(),
        Outcome::Skipped => engine.stats.skipped(),
    }
    engine.succeeded();
}

pub(super) enum Decision {
    Stop,
    /// The one retry of a transient failure, after this pause.
    Retry(Duration),
    /// The peer is too busy: again after this pause, without using the retry.
    Wait(Duration),
    Access,
    Fatal(String),
    Report,
}

pub(super) fn decide(
    engine: &Engine<'_>,
    file: &mut FileWork,
    failure: &OpError,
    asked_access: bool,
) -> Decision {
    if engine.stopped() {
        return Decision::Stop;
    }
    let kind = failure.error.kind();
    if failure.at == At::Source
        && kind == io::ErrorKind::PermissionDenied
        && engine.gate.is_some()
        && !asked_access
    {
        return Decision::Access;
    }
    let overload = classify_error(&failure.error) == OpOutcome::Overload;
    let before_publication = matches!(failure.at, At::Source | At::Target);
    if before_publication && is_back_pressure(kind, overload) {
        return match overload_wait(engine, file, &failure.error) {
            Some(delay) => Decision::Wait(delay),
            None => Decision::Report,
        };
    }
    if before_publication && !file.retried && is_transient(kind, overload) {
        return Decision::Retry(retry_delay(None, super::jitter()));
    }
    if failure.at == At::Target && ends_job_at_target(kind) {
        return Decision::Fatal(target_refuses(&failure.error));
    }
    Decision::Report
}

/// Back-pressure is no failure of the file (K13): it waits for the peer's
/// own delay, without any permit, as long as the job still moves; `None`
/// once the patience ran out without progress.
pub(super) fn overload_wait(
    engine: &Engine<'_>,
    file: &mut FileWork,
    error: &io::Error,
) -> Option<Duration> {
    let since = *file.overloaded_since.get_or_insert_with(Instant::now);
    if !overload_keeps_waiting(engine.stats.quiet_since(since)) {
        return None;
    }
    let retry_after =
        crate::vfs::congestion_of(error).and_then(|congestion| congestion.retry_after);
    Some(retry_delay(retry_after, super::jitter()))
}

/// Why a job ends at a target that takes nothing more.
pub(super) fn target_refuses(error: &io::Error) -> String {
    format!("Das Ziel nimmt keine Dateien mehr an – Übertragung beendet: {error}")
}

/// Asks once per job for read access to protected local folders, outside of
/// every permit; true when the file should be tried again.
fn ask_access(engine: &Engine<'_>, file: &FileWork, failure: &OpError) -> bool {
    let Some(gate) = engine.gate.as_ref() else {
        return false;
    };
    match gate.request() {
        AccessAnswer::Granted => true,
        AccessAnswer::Refused => {
            engine.fatal(
                "Lesezugriff wurde abgelehnt – Übertragung beendet; bereits Übertragenes bleibt erhalten"
                    .to_string(),
            );
            false
        }
        AccessAnswer::Unavailable(detail) => {
            let error = super::super::access::with_access_detail(
                io::Error::new(failure.error.kind(), failure.error.to_string()),
                detail.as_deref(),
            );
            failed(engine, &file.source, &OpError::source(error));
            false
        }
    }
}

/// Reports a file that was not transferred; a long run of connection
/// failures ends the job instead of collecting thousands of them.
pub(super) fn failed(engine: &Engine<'_>, path: &str, failure: &OpError) {
    report(engine, path, failure);
    if is_connection_failure(failure) {
        breaker(engine, &failure.error);
    }
}

/// Reports a file without feeding the breaker (the rest of a packet whose
/// one connection failure counted already).
pub(super) fn report(engine: &Engine<'_>, path: &str, failure: &OpError) {
    let message = match failure.at {
        At::Unknown => format!(
            "Ergebnis unbekannt – die Datei kann am Ziel angelegt worden sein und wird nicht erneut übertragen: {}",
            failure.error
        ),
        _ => failure.error.to_string(),
    };
    engine.issue(path, &message);
}

/// Whether `failure` speaks about the connection rather than the one file.
pub(super) fn is_connection_failure(failure: &OpError) -> bool {
    let overload = classify_error(&failure.error) == OpOutcome::Overload;
    connection_failure(failure.error.kind(), overload)
}

/// Counts one connection failure; a run of them ends the job.
pub(super) fn breaker(engine: &Engine<'_>, detail: &dyn std::fmt::Display) {
    if engine.connection_failed() {
        engine.fatal(format!(
            "Übertragung beendet: viele Fehler in Folge ohne einen Erfolg – die Verbindung scheint unterbrochen ({detail})"
        ));
    }
}
