//! The time plan of a screen reconnection and its attempts (moved out of
//! `screen_ipc_reconnect.rs` for plan 2026-11-17 P9, which keeps that file
//! under 500 lines): at most one attempt per `RECONNECT_EVERY`, none
//! `RECONNECT_FOR` after the last attempt that got further than all before
//! it (`Schedule::failed`).
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::{NOT_READY, RECONNECT_EVERY, RECONNECT_FOR};
use crate::screen::core::viewer_client::ViewerError;

/// What the reconnection does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum Step {
    /// One attempt now, with at most `remaining` for it.
    Attempt { remaining: Duration },
    /// The next attempt is due after this time.
    Wait(Duration),
    /// `RECONNECT_FOR` passed without progress.
    GiveUp,
}

/// The time plan of one reconnection: at most one attempt per
/// `RECONNECT_EVERY`, none `RECONNECT_FOR` after the last progress.
pub(in super::super) struct Schedule {
    start: Instant,
    next: Instant,
    /// The furthest stage an attempt reached (`stage`).
    reached: u8,
    attempts: u32,
    /// Code and detail of the last failed attempt.
    last: Option<(String, String)>,
}

/// How far a failed attempt got: 1 the device was not reached or the link
/// broke, 2 the host answered (its session not ready yet).
fn stage(error: &ViewerError) -> u8 {
    match error {
        ViewerError::Io(_) => 1,
        ViewerError::Refused(_) | ViewerError::Unsupported => 2,
    }
}

impl Schedule {
    pub(in super::super) fn new(now: Instant) -> Self {
        Self {
            start: now,
            next: now,
            reached: 0,
            attempts: 0,
            last: None,
        }
    }

    /// An attempt failed with `error` at `now`; one that got further than
    /// every attempt before it is progress, the plan counts from it.
    pub(in super::super) fn failed(&mut self, now: Instant, error: &ViewerError) {
        self.attempts = self.attempts.saturating_add(1);
        let stage = stage(error);
        if stage > self.reached {
            self.reached = stage;
            self.start = self.start.max(now);
        }
        self.last = Some((error.code(), error.detail()));
    }

    /// The end of a plan without progress: the last failure's code and its
    /// detail with the number of attempts.
    pub(in super::super) fn gave_up(&self) -> Option<(String, String)> {
        let (code, detail) = self.last.clone()?;
        Some((
            code,
            format!(
                "{detail} (Verbindungsaufbau nach {} Versuchen ohne Fortschritt beendet)",
                self.attempts
            ),
        ))
    }

    pub(in super::super) fn step(&mut self, now: Instant) -> Step {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= RECONNECT_FOR {
            return Step::GiveUp;
        }
        if now < self.next {
            return Step::Wait(self.next - now);
        }
        self.next = now + RECONNECT_EVERY;
        Step::Attempt {
            remaining: RECONNECT_FOR - elapsed,
        }
    }
}

/// A future the reconnection waits for.
pub(in super::super) type Pending<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The clock of the reconnection (a simulated one in the task tests).
pub(in super::super) trait Clock: Send {
    fn now(&self) -> Instant;
    fn sleep(&self, time: Duration) -> Pending<()>;
}

/// The clock of the system.
pub(in super::super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, time: Duration) -> Pending<()> {
        Box::pin(tokio::time::sleep(time))
    }
}

/// One new connection and handshake to the same device.
pub(in super::super) trait Dial: Send {
    type Session: Send + 'static;
    fn dial(&self, remaining: Duration) -> Pending<Result<Self::Session, ViewerError>>;
}

/// How a reconnection ended.
pub(in super::super) enum Outcome<S> {
    Opened(S),
    Refused(ViewerError),
    GaveUp,
    ViewerClosed,
}

/// Whether a failed attempt may be repeated: the device was not reached, or
/// its new session is not ready yet (`NOT_READY`). Any other refusal of the
/// host is final, and so is a host whose identity does not match the pinned
/// one (`PermissionDenied`, P9: a new connection cannot change that).
pub(super) fn retryable(error: &ViewerError) -> bool {
    match error {
        ViewerError::Io(error) => error.kind() != std::io::ErrorKind::PermissionDenied,
        ViewerError::Refused(code) => NOT_READY.iter().any(|ready| ready.key() == code.as_str()),
        ViewerError::Unsupported => false,
    }
}

/// Waits for `wait`; `None` when the client closed meanwhile. Control frames
/// of the client arriving now are not sent anywhere; the choices among them
/// go into `modes`, which the next session gets (`ModeMemory::note`).
async fn unless_closed<T>(
    mut wait: Pending<T>,
    input: &mut mpsc::Receiver<Option<super::RelayedFrame>>,
    modes: &mut super::ModeMemory,
) -> Option<T> {
    loop {
        tokio::select! {
            biased;
            frame = input.recv() => {
                let Some(Some(frame)) = frame else {
                    return None;
                };
                // Review 2026-11-19 (Gemini 3/9): a choice the viewer made
                // meanwhile reaches the next session with the others.
                modes.note_relayed(&frame);
            }
            value = &mut wait => return Some(value),
        }
    }
}

/// Builds the session again after `session_changed` by the plan `schedule`:
/// one dial per attempt.
pub(in super::super) async fn again<D: Dial, C: Clock>(
    dialer: D,
    clock: C,
    schedule: &mut Schedule,
    input: &mut mpsc::Receiver<Option<super::RelayedFrame>>,
    modes: &mut super::ModeMemory,
) -> Outcome<D::Session> {
    loop {
        match schedule.step(clock.now()) {
            Step::GiveUp => return Outcome::GaveUp,
            Step::Wait(time) => {
                if unless_closed(clock.sleep(time), input, modes)
                    .await
                    .is_none()
                {
                    return Outcome::ViewerClosed;
                }
            }
            Step::Attempt { remaining } => {
                match unless_closed(dialer.dial(remaining), input, modes).await {
                    None => return Outcome::ViewerClosed,
                    Some(Ok(session)) => return Outcome::Opened(session),
                    Some(Err(error)) if retryable(&error) => {
                        crate::screen::os::host_worker_client::host_log::note(
                            None,
                            &format!("screen connect attempt failed: {}", error.detail()),
                        );
                        schedule.failed(clock.now(), &error);
                    }
                    Some(Err(error)) => return Outcome::Refused(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p9_task_reconnect_plan_counts_from_progress() {
        let start = Instant::now();
        let mut plan = Schedule::new(start);
        assert!(matches!(plan.step(start), Step::Attempt { .. }));
        let unreachable = ViewerError::Io(std::io::Error::other("Verbindung zum Gerät: offline"));
        plan.failed(start, &unreachable);
        // The device stays unreachable: the same stage is no progress.
        let later = start + RECONNECT_FOR - Duration::from_secs(1);
        plan.failed(later, &unreachable);
        assert_eq!(plan.step(start + RECONNECT_FOR), Step::GiveUp);

        let mut plan = Schedule::new(start);
        plan.failed(start, &unreachable);
        // The host answers late: progress, the plan counts from it.
        plan.failed(later, &ViewerError::Refused("capture_failed".into()));
        assert!(matches!(
            plan.step(start + RECONNECT_FOR),
            Step::Attempt { .. }
        ));
        assert_eq!(plan.step(later + RECONNECT_FOR), Step::GiveUp);
        let (code, detail) = plan.gave_up().expect("a failed attempt");
        assert_eq!(code, "capture_failed");
        assert!(detail.contains("nach 2 Versuchen"), "{detail}");
        assert!(Schedule::new(start).gave_up().is_none());
    }
}
