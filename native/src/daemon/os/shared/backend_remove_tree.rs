//! `RemoveTree` (`ext-v1`) for the backend the background service serves:
//! the served backend deletes the whole folder itself (`vfs::remove_tree`;
//! for a Share peer one request its host runs next to the data), so the
//! client sends one frame instead of a walk with one request per entry.
//! The counters go out as `Progress` at most every 250 ms with a final
//! report; a cancel of the request, or a client that no longer takes the
//! reports, reaches the backend's run at its next entry.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::agent_proto::{Frame, UNSUPPORTED_EXTENSION};
use crate::vfs::{self as vfs, BackendHandle, RecursiveDeleteStatus};

use super::backend_server::{emit, Sink};

/// Shortest interval between two progress reports.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// `RemoveTree`: `Progress`*, then `End`. A failure sends the counters
/// reached as a last `Progress` and returns the error (the request worker
/// answers it with `Err`); a cancel returns `Interrupted` like every other
/// canceled request of the service.
pub(super) fn remove_tree(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !vfs::supports_remove_tree(&**backend, path)? {
        return emit(sink, id, &Frame::Err(UNSUPPORTED_EXTENSION.to_string()));
    }
    let mut last: Option<Instant> = None;
    let mut reporting = true;
    let outcome = vfs::remove_tree(&**backend, path, cancel, &mut |progress| {
        if !reporting || last.is_some_and(|last| last.elapsed() < PROGRESS_EVERY) {
            return;
        }
        last = Some(Instant::now());
        let frame = Frame::Progress {
            done: progress.removed,
            total: progress.planned,
        };
        if emit(sink, id, &frame).is_err() {
            // The client is gone: stop the backend's run at its next entry.
            reporting = false;
            cancel.store(true, Ordering::Relaxed);
        }
    });
    match outcome {
        Ok(report) if report.status == RecursiveDeleteStatus::Complete => {
            let frame = Frame::Progress {
                done: report.removed,
                total: report.planned,
            };
            emit(sink, id, &frame)?;
            emit(sink, id, &Frame::End)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "folder deletion canceled",
        )),
        Err(failure) => {
            if reporting {
                // The failure is the answer; a lost report changes nothing.
                let frame = Frame::Progress {
                    done: failure.removed,
                    total: failure.planned,
                };
                let _ = emit(sink, id, &frame);
            }
            Err(failure.error)
        }
    }
}

#[cfg(test)]
#[path = "delete_speed_task_tests.rs"]
mod delete_speed_task_tests;
