//! One release-only input job retains its original worker while capture is in flight.
use super::*;
use crate::screen::core::input_model::MAX_INPUT_EVENTS;
use crate::screen::core::wire_control::ControlBlock;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Runs one input injection on tokio's blocking pool and waits for it. A
/// host process answers input only after a running picture request (the
/// worker sink waits up to `INPUT_TIMEOUT` for the link, then as long for
/// the answer); tokio 1.52.3: "issuing a blocking call ... in a future
/// without yielding is problematic, as it may prevent the executor from
/// driving other futures forward", `spawn_blocking` "runs the provided
/// closure on a thread dedicated to blocking operations". The session still
/// waits for the result, so input keeps its order and its control state.
pub(super) async fn inject_off_runtime(
    inject: impl FnOnce() -> Result<(), ControlBlock> + Send + 'static,
) -> Result<(), ControlBlock> {
    tokio::task::spawn_blocking(inject)
        .await
        .unwrap_or(Err(ControlBlock::ControlUnavailable))
}

/// What the session keeps of input that did not go as asked (review
/// 2026-11-19).
#[derive(Default)]
pub(super) struct InputHealth {
    /// D7: release events whose injection failed. The release job puts them
    /// here itself, so they stay whoever awaits the job (`poll_input_release`
    /// or `family_operation`); the platform's `release` leaves failed
    /// releases to its caller (`input_send::release`).
    failed: Arc<Mutex<Vec<InputEvent>>>,
    /// D6: the block the viewer's last `CONTROL_STATE` named.
    pub(super) block_reported: Option<ControlBlock>,
    /// E2: the viewer heard that input is held back, once per hold.
    pub(super) held_reported: bool,
    /// Z2: a block the platform reported late (`take_refusal`) stays shown
    /// until then, also when the next batch goes through.
    pub(super) block_shown_until: Option<Instant>,
    /// D2: a batch refused as `control_unavailable` that may only have named
    /// a reference the source left meanwhile: its reference and the picture
    /// operations started when it was refused (`Session::stale_refusal`).
    pub(super) unconfirmed: Option<(u64, u64)>,
}

fn failed_list(failed: &Mutex<Vec<InputEvent>>) -> MutexGuard<'_, Vec<InputEvent>> {
    failed.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Release events on their way: unless their release succeeded they go
/// back to `InputHealth::failed`, also when the release panicked.
struct Retained {
    failed: Arc<Mutex<Vec<InputEvent>>>,
    events: Vec<InputEvent>,
    released: bool,
}
impl Drop for Retained {
    fn drop(&mut self) {
        if !self.released {
            failed_list(&self.failed).append(&mut self.events);
        }
    }
}

impl Session<'_> {
    pub(super) fn release_pressed(&mut self) {
        let released = self.pressed.release_all();
        // Input is held while this job exists, so a second batch cannot grow
        // behind it; what is held now goes after it (Gemini 3/10: it was
        // left held), with the next retry or the release of the control.
        if self.input_release.is_some() {
            failed_list(&self.health.failed).extend(released);
            return;
        }
        self.start_release(released);
    }
    /// Gemini 3/10: the releases of keys and buttons the host holds go in
    /// also while input waits (a hidden view, no presented picture yet, a
    /// release first): they carry no position, and the viewer sends them
    /// once only.
    pub(super) fn release_held_ups(&mut self, events: &[InputEvent]) {
        let ups: Vec<InputEvent> = events
            .iter()
            .filter(|event| self.pressed.take_release(event))
            .cloned()
            .collect();
        if self.input_release.is_some() {
            failed_list(&self.health.failed).extend(ups);
        } else {
            self.start_release(ups);
        }
    }
    /// One release job for `events`; what fails goes back to
    /// `InputHealth::failed` (`Retained`).
    fn start_release(&mut self, events: Vec<InputEvent>) {
        if events.is_empty() {
            return;
        }
        let mut retained = Retained {
            failed: self.health.failed.clone(),
            events,
            released: false,
        };
        // Both ways run on the blocking pool (review 2026-11-19, M1): a source
        // without a sink (Android) injects with the backend; its queue no
        // longer waits for room (`bridge_input_queue`: merged, refused beyond
        // `HARD_LIMIT`), but a platform injection may still block. The
        // session waits for the job as for every release: input is held
        // while it exists (`input`), so later batches follow the releases.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let job: Box<dyn Fn(&[InputEvent]) -> Result<(), ControlBlock> + Send> =
                match self.pipeline.input_sink() {
                    Some(sink) => Box::new(move |events| sink.release(events)),
                    None => {
                        let backend = self.context.backend.clone();
                        Box::new(move |events| backend.inject(events))
                    }
                };
            self.input_release = Some(runtime.spawn_blocking(move || {
                // Gemini 3/10: a host process takes at most `MAX_INPUT_EVENTS`
                // per batch (`encode_batch`); what went in leaves `retained`.
                let mut done = 0;
                let mut result = Ok(());
                for chunk in retained.events.chunks(MAX_INPUT_EVENTS) {
                    result = job(chunk);
                    if result.is_err() {
                        break;
                    }
                    done += chunk.len();
                }
                retained.events.drain(..done);
                retained.released = result.is_ok();
                result
            }));
            return;
        }
        let mut done = 0;
        let mut result = Ok(());
        for chunk in retained.events.chunks(MAX_INPUT_EVENTS) {
            result = self.context.backend.inject(chunk);
            if result.is_err() {
                break;
            }
            done += chunk.len();
        }
        retained.events.drain(..done);
        self.release_blocked = result.err();
        retained.released = self.release_blocked.is_none();
    }
    /// D7: the releases that failed go out once more, alone: what the user
    /// holds now (a drag, Shift) stays held (review O2b). `true` when there
    /// were any. Not on every reference check (`refresh_input_reference`):
    /// only with the next input batch and with the release of the control,
    /// so a lasting block (the secure desktop) costs no release attempt per
    /// picture.
    pub(super) fn retry_releases(&mut self) -> bool {
        if self.input_release.is_some() {
            return false;
        }
        let failed = std::mem::take(&mut *failed_list(&self.health.failed));
        if failed.is_empty() {
            return false;
        }
        self.start_release(failed);
        true
    }
    /// The release of the control: everything held now (newest first) and
    /// the releases that failed before, in one job.
    pub(super) fn release_all_and_failed(&mut self) {
        // A job still running (one that outlasted `CLOSE_GRACE`, E4): what
        // is held now waits behind it with the failed releases.
        if self.input_release.is_some() {
            let held = self.pressed.release_all();
            failed_list(&self.health.failed).extend(held);
            return;
        }
        let mut events = self.pressed.release_all();
        events.append(&mut failed_list(&self.health.failed));
        self.start_release(events);
    }
    /// Z3/Z4: waits for the running release job, at most `CLOSE_GRACE`; a
    /// job that takes longer stays the session's (E4: input keeps waiting
    /// behind it, `poll_input_release` takes its result later; no block is
    /// made up for it), and the host log names it.
    pub(super) async fn finish_release(&mut self) {
        let Some(mut job) = self.input_release.take() else {
            return;
        };
        match tokio::time::timeout(CLOSE_GRACE, &mut job).await {
            Ok(Ok(result)) => self.release_blocked = result.err(),
            Ok(Err(_)) => self.release_blocked = Some(ControlBlock::ControlUnavailable),
            Err(_) => {
                super::visibility::note_viewer(
                    "the release of held input outlasted the close grace; the control goes on",
                );
                self.input_release = Some(job);
            }
        }
    }
    pub(super) fn poll_input_release(&mut self) {
        let Some(mut job) = self.input_release.take() else {
            return;
        };
        let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        match std::pin::Pin::new(&mut job).poll(&mut cx) {
            std::task::Poll::Pending => self.input_release = Some(job),
            std::task::Poll::Ready(Ok(result)) => self.release_blocked = result.err(),
            std::task::Poll::Ready(Err(_)) => {
                self.release_blocked = Some(ControlBlock::ControlUnavailable)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inject_off_runtime;
    use crate::screen::core::wire_control::ControlBlock;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// Input that a host process holds back behind a running picture
    /// request blocks a blocking-pool thread, not the session's runtime:
    /// on a current-thread runtime the task that ends the wait still runs
    /// and the input then arrives. A panicking platform injection becomes a
    /// blocked control instead of a dead session task.
    #[test]
    fn screen_metadata_runtime_task_input_injection_leaves_the_runtime_free() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            let waiting = gate.clone();
            let injected = inject_off_runtime(move || {
                let (open, changed) = &*waiting;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
                Ok(())
            });
            let opener = tokio::spawn(async move {
                let (open, changed) = &*gate;
                *open.lock().unwrap() = true;
                changed.notify_all();
            });
            let result = tokio::time::timeout(Duration::from_secs(10), injected).await;
            assert_eq!(result.expect("the runtime kept running"), Ok(()));
            opener.await.unwrap();
            let failed = inject_off_runtime(|| panic!("platform input panicked")).await;
            assert_eq!(failed, Err(ControlBlock::ControlUnavailable));
        });
    }
}
