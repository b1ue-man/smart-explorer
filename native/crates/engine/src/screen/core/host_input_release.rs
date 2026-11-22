//! One release-only input job retains its original worker while capture is in flight.
use super::*;
use crate::screen::core::input_model::MAX_INPUT_EVENTS;
use crate::screen::core::wire_control::ControlBlock;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[path = "host_release_gate.rs"]
pub(super) mod gate;

/// Runs one input injection on tokio's blocking pool and waits for it. A
/// host process answers input only after a running picture request (the
/// worker sink waits up to `INPUT_TIMEOUT` for the link, then as long for
/// the answer); tokio 1.52.3: "issuing a blocking call ... in a future
/// without yielding is problematic, as it may prevent the executor from
/// driving other futures forward", `spawn_blocking` "runs the provided
/// closure on a thread dedicated to blocking operations". The session still
/// waits for the result, so input keeps its order and its control state.
#[cfg_attr(not(test), allow(dead_code))] // the session spawns its own (Codex 7/10)
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
    /// Releases waiting for the next job (one runs); session thread only.
    pub(super) queued: Vec<InputEvent>,
    /// The last new try of failed releases (`retry_failed_releases`).
    retried: Option<Instant>,
    /// A hidden view or a changed source asked for a revocation while a job
    /// without one ran; the next job revokes.
    revoke_pending: bool,
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
    /// Codex 7/10: the batch being injected off the session loop, and the
    /// batches that wait behind it in their order.
    pub(super) injecting: Option<Injection>,
    /// Round 10: the backlog overflowed; only releases queue until it drained.
    pub(super) overflow: bool,
    /// Round 8 (I5): counts every release of all held keys; a batch whose
    /// injection began before one is let go again when it ends.
    pub(super) release_epoch: u64,
    pub(super) backlog: std::collections::VecDeque<Vec<InputEvent>>,
}

/// One batch on its way to the platform (`Session::start_batch`).
pub(super) struct Injection {
    pub(super) handle: tokio::task::JoinHandle<Result<(), ControlBlock>>,
    pub(super) events: Vec<InputEvent>,
    pub(super) reference: u64,
    pub(super) presented: bool,
    pub(super) started: Instant,
    /// `InputHealth::release_epoch` when it began.
    pub(super) epoch: u64,
}

/// How often failed releases are tried again (a lasting refusal, the secure
/// desktop, costs no attempt per picture).
const RELEASE_RETRY: std::time::Duration = std::time::Duration::from_secs(1);

/// `event` lets go of a key, button or touch.
pub(super) fn releases(event: &InputEvent) -> bool {
    matches!(
        event,
        InputEvent::Key { down: false, .. }
            | InputEvent::Button { down: false, .. }
            | InputEvent::Touch {
                phase: crate::screen::core::input_model::TouchPhase::End
                    | crate::screen::core::input_model::TouchPhase::Cancel,
                ..
            }
    )
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

/// The release invariant (review 2026-11-19, round 6): every release of a
/// key, button or touch this session holds (`pressed`) or owes (`queued`,
/// `failed`) reaches the sink exactly once and as early as it can, whatever
/// the control, the family mode, a running job, a timeout or the end of the
/// session. One way sends them: `flush_releases`, one job at a time; what a
/// job could not deliver comes back to `failed` (`Retained`), what arrives
/// while one runs waits in `queued` and goes with the next job, which
/// `poll_input_release` starts as soon as the running one ended.
impl Session<'_> {
    /// Everything held now is let go (a new reference, a refused batch).
    pub(super) fn release_pressed(&mut self) {
        let released = self.pressed.release_all();
        self.health.queued.extend(released);
        self.health.release_epoch += 1;
        self.flush_releases();
    }
    /// `release_pressed` for a view hidden or a source changed (round 7):
    /// input of this session still waiting in a queueing sink is taken back
    /// as well (`ScreenInputSink::revoke`), also with nothing held.
    pub(super) fn release_pressed_revoking(&mut self) {
        let released = self.pressed.release_all();
        self.health.queued.extend(released);
        // A job without the revocation runs: the next job revokes (whoever
        // starts it: `poll_input_release`, `drain_releases`, a family path).
        if self.input_release.is_some() || self.health.injecting.is_some() {
            self.health.revoke_pending = true;
        }
        self.health.release_epoch += 1;
        self.flush_releases_as(true);
    }
    /// Gemini 3/10, E2: the releases of keys and buttons the host holds go
    /// in from every batch, also one that is held back, refused or not this
    /// session's turn: they carry no position, and the viewer sends each once.
    pub(super) fn release_held_ups(&mut self, events: &[InputEvent]) {
        let ups: Vec<InputEvent> = events
            .iter()
            .filter(|event| self.pressed.take_release(event))
            .cloned()
            .collect();
        self.health.queued.extend(ups);
        self.flush_releases();
    }
    /// The one way releases go: the failed ones (older) and the queued ones
    /// in one job, unless a job runs (`poll_input_release` starts the next
    /// one when it ends). `true` while a job runs.
    pub(super) fn flush_releases(&mut self) -> bool {
        self.flush_releases_as(false)
    }
    /// `flush_releases`; `revoke`: the session gives the control up or ends
    /// (`drain_releases`), so a queueing sink also takes back what of it
    /// still waits (`ScreenInputSink::revoke`, round 7). Every other release
    /// (a held-back batch's ups, a new reference) keeps the session's input.
    fn flush_releases_as(&mut self, revoke: bool) -> bool {
        // Releases never overtake a batch being injected (Codex 7/10):
        // `settle_input` flushes them when it ended.
        if self.input_release.is_none() && self.health.injecting.is_none() {
            let revoke = revoke || std::mem::take(&mut self.health.revoke_pending);
            let mut events = std::mem::take(&mut *failed_list(&self.health.failed));
            events.append(&mut self.health.queued);
            // A revocation goes also with nothing to release (a nudge).
            self.start_release(events, revoke, revoke);
        }
        self.input_release.is_some()
    }
    /// Releases wait to be delivered (failed or queued); input waits behind
    /// them (N2).
    pub(super) fn releases_owed(&self) -> bool {
        !self.health.queued.is_empty() || !failed_list(&self.health.failed).is_empty()
    }
    /// Failed releases are tried again at most once per `RELEASE_RETRY`
    /// (from `poll_input_release`, on every viewer frame and capture tick),
    /// with or without the control: they are this session's own.
    fn retry_failed_releases(&mut self) {
        let due = self
            .health
            .retried
            .is_none_or(|at| at.elapsed() >= RELEASE_RETRY);
        if due && !failed_list(&self.health.failed).is_empty() {
            self.health.retried = Some(Instant::now());
            self.flush_releases();
        }
    }
    /// One release job for `events`; what fails goes back to
    /// `InputHealth::failed` (`Retained`). `nudge`: also without events when
    /// the source has a sink, so a sink that owes releases of its own (the
    /// Wayland portal's refused ones) can deliver them (round 6, R3).
    fn start_release(&mut self, events: Vec<InputEvent>, nudge: bool, revoke: bool) {
        let sink = self.pipeline.input_sink();
        if events.is_empty() && !(nudge && sink.is_some()) {
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
            let job: Box<dyn Fn(&[InputEvent]) -> Result<(), ControlBlock> + Send> = match sink {
                Some(sink) if revoke => Box::new(move |events| sink.revoke(events)),
                Some(sink) => Box::new(move |events| sink.release(events)),
                None => {
                    let backend = self.context.backend.clone();
                    Box::new(move |events| backend.inject(events))
                }
            };
            self.input_release = Some(runtime.spawn_blocking(move || {
                // Gemini 3/10: a host process takes at most `MAX_INPUT_EVENTS`
                // per batch (`encode_batch`); what went in leaves `retained`.
                if retained.events.is_empty() {
                    // A nudge: the sink's own debts; it reports nothing of ours.
                    retained.released = true;
                    let _ = job(&[]);
                    return Ok(());
                }
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
    /// Lets go of everything this session holds or owes and waits for it,
    /// at most `CLOSE_GRACE` per job (giving up the control, the end of the
    /// session; round 6, R1): the running job, then one job with all that is
    /// held, queued and failed (on an empty list a nudge for the sink's own
    /// debts), then once more what that one could not deliver. A job that
    /// outlasts its grace goes on, and the host log names it.
    pub(super) async fn drain_releases(&mut self) {
        self.finish_injection().await;
        self.finish_release().await;
        let held = self.pressed.release_all();
        self.health.queued.extend(held);
        if !self.flush_releases_as(true) && self.health.injecting.is_none() {
            self.start_release(Vec::new(), true, true);
        }
        self.finish_release().await;
        if self.input_release.is_none() && self.flush_releases_as(true) {
            self.finish_release().await;
        }
    }
    /// Codex 7/10: before the control goes, the batch being injected ends
    /// (at most `CLOSE_GRACE`); its keys count as held then, so the release
    /// that follows lets go of them, and the batches waiting behind it are
    /// dropped (their releases are part of that release).
    async fn finish_injection(&mut self) {
        self.health.backlog.clear();
        let Some(mut job) = self.health.injecting.take() else {
            return;
        };
        let applied = match tokio::time::timeout(CLOSE_GRACE, &mut job.handle).await {
            Ok(result) => result.map(|result| result.is_ok()).unwrap_or(false),
            Err(_) => {
                super::visibility::note_viewer("an input injection outlasted the close grace");
                // Round 8 (I8): it stays the session's, so no release runs
                // beside it; `hand_releases_over` gives it to the keeper.
                // Round 9 (E5): only its presses count as held; a release in
                // it may not have gone, so what it releases stays held and is
                // let go again (a second release is harmless).
                for event in job.events.iter().filter(|event| !releases(event)) {
                    self.pressed.observe(event);
                }
                self.health.injecting = Some(job);
                return;
            }
        };
        if applied {
            for event in &job.events {
                self.pressed.observe(event);
            }
        }
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
                    "the release of held input outlasted the close grace; it goes on",
                );
                self.input_release = Some(job);
            }
        }
    }
    /// The running job's end; releases queued meanwhile go at once in the
    /// next job (failed ones wait for `retry_failed_releases`, so a lasting
    /// refusal costs no attempt per picture).
    pub(super) fn poll_input_release(&mut self) {
        let Some(mut job) = self.input_release.take() else {
            self.retry_failed_releases();
            return;
        };
        let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        match std::pin::Pin::new(&mut job).poll(&mut cx) {
            std::task::Poll::Pending => {
                self.input_release = Some(job);
                return;
            }
            std::task::Poll::Ready(Ok(result)) => self.release_blocked = result.err(),
            std::task::Poll::Ready(Err(_)) => {
                self.release_blocked = Some(ControlBlock::ControlUnavailable)
            }
        }
        if !self.health.queued.is_empty() || self.health.revoke_pending {
            self.flush_releases();
        }
    }
    /// Codex 7/1: what is still on its way (the running job, failed and
    /// queued releases) leaves the session for the release keeper, which
    /// delivers it while every session's input waits (`gate::busy`).
    /// The input target of this session's source (`ScreenInputSink::input_target`;
    /// the backend reaches the same desktop).
    pub(super) fn input_target(&self) -> String {
        self.pipeline
            .input_sink()
            .map_or_else(|| "desktop".into(), |sink| sink.input_target())
    }
    /// Releases of a former holder of this session's target are on their way.
    pub(super) fn target_busy(&self) -> bool {
        gate::busy(&self.input_target())
    }
    pub(super) fn hand_releases_over(&mut self) {
        // An injection that outlasted its grace goes first (round 8, I8).
        let jobs: Vec<_> = (self.health.injecting.take().map(|job| job.handle))
            .into_iter()
            .chain(self.input_release.take())
            .collect();
        let queued = std::mem::take(&mut self.health.queued);
        if jobs.is_empty() && queued.is_empty() && failed_list(&self.health.failed).is_empty() {
            return;
        }
        let key = self.input_target();
        let target = match self.pipeline.input_sink() {
            Some(sink) => gate::Target::Sink(sink),
            None => gate::Target::Backend(self.context.backend.clone()),
        };
        // Round 8 (I3): the keeper owns this list now; the session goes on
        // with a new one, so it never takes the keeper's releases.
        let failed = std::mem::take(&mut self.health.failed);
        gate::take_over(jobs, failed, queued, target, key);
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
