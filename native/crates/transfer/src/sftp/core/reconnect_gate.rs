use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const STATE_LOCK_RETRY_MAX: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Debug)]
pub(super) struct AbsoluteDeadline {
    expires: Instant,
}

impl AbsoluteDeadline {
    pub(super) fn after(timeout: Duration) -> Self {
        Self {
            expires: Instant::now() + timeout,
        }
    }

    pub(super) fn expires(self) -> Instant {
        self.expires
    }

    pub(super) fn remaining(self, stage: &str) -> io::Result<Duration> {
        self.remaining_at(Instant::now(), stage)
    }

    pub(super) fn timeout(self, stage: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("SFTP metadata deadline expired during {stage}"),
        )
    }

    fn remaining_at(self, now: Instant, stage: &str) -> io::Result<Duration> {
        self.expires
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| self.timeout(stage))
    }
}

pub(in crate::sftp) struct Generation<T> {
    value: T,
    stale: AtomicBool,
}

impl<T> Generation<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            stale: AtomicBool::new(false),
        }
    }

    pub(super) fn value(&self) -> &T {
        &self.value
    }

    pub(super) fn mark_stale(&self) {
        self.stale.store(true, Ordering::Release);
    }

    pub(super) fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Acquire)
    }
}

struct ReconnectState<T> {
    current: Arc<Generation<T>>,
    reconnecting: bool,
}

pub(super) enum ReconnectAccess<'a, T> {
    Current(Arc<Generation<T>>),
    Reconnect(ReconnectPermit<'a, T>),
}

pub(super) struct ReconnectPermit<'a, T> {
    gate: &'a ReconnectGate<T>,
    active: bool,
}

/// Serializes transport replacement without holding the state mutex across
/// network setup. Deadline-bound callers wait on the same in-flight attempt
/// only until their own absolute deadline; they never queue behind repeated
/// full connect timeouts. The state mutex is an invariant-only critical
/// section: holders may clone/swap the generation and update flags, but must
/// release it before network work or sleeping (a Condvar wait releases it).
pub(super) struct ReconnectGate<T> {
    state: Mutex<ReconnectState<T>>,
    ready: Condvar,
}

impl<T> ReconnectGate<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            state: Mutex::new(ReconnectState {
                current: Arc::new(Generation::new(value)),
                reconnecting: false,
            }),
            ready: Condvar::new(),
        }
    }

    pub(super) fn acquire(
        &self,
        deadline: Option<AbsoluteDeadline>,
        usable: impl Fn(&Generation<T>) -> bool,
    ) -> io::Result<ReconnectAccess<'_, T>> {
        if let Some(deadline) = deadline {
            deadline.remaining("reconnect-state acquisition")?;
        }
        let mut state =
            lock_with_deadline(&self.state, deadline, "reconnect-state lock acquisition")?;
        loop {
            if let Some(deadline) = deadline {
                deadline.remaining("reconnect-state acquisition")?;
            }
            let current = state.current.clone();
            if usable(&current) {
                return Ok(ReconnectAccess::Current(current));
            }
            current.mark_stale();
            if !state.reconnecting {
                state.reconnecting = true;
                return Ok(ReconnectAccess::Reconnect(ReconnectPermit {
                    gate: self,
                    active: true,
                }));
            }
            state = self.wait(state, deadline)?;
        }
    }

    fn finish_reconnect(
        &self,
        deadline: Option<AbsoluteDeadline>,
        replacement: io::Result<T>,
    ) -> io::Result<Arc<Generation<T>>> {
        let mut state =
            match lock_with_deadline(&self.state, deadline, "reconnect-state publication") {
                Ok(state) => state,
                Err(error) => {
                    // Publication timed out, but leaving the gate claimed would
                    // strand every later caller. No gate holder performs I/O or
                    // sleeps, so this fallback is only the bounded flag repair.
                    self.abandon_reconnect();
                    return Err(error);
                }
            };
        state.reconnecting = false;
        let result = replacement.map(|replacement| {
            let generation = Arc::new(Generation::new(replacement));
            state.current = generation.clone();
            generation
        });
        self.ready.notify_all();
        result
    }

    fn abandon_reconnect(&self) {
        let mut state = lock_unpoisoned(&self.state);
        state.reconnecting = false;
        self.ready.notify_all();
    }

    fn wait<'a>(
        &self,
        state: MutexGuard<'a, ReconnectState<T>>,
        deadline: Option<AbsoluteDeadline>,
    ) -> io::Result<MutexGuard<'a, ReconnectState<T>>> {
        match deadline {
            Some(deadline) => {
                let remaining = deadline.remaining("reconnect-gate wait")?;
                let (state, timeout) = self
                    .ready
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if timeout.timed_out() {
                    return Err(deadline.timeout("reconnect-gate wait"));
                }
                deadline.remaining("reconnect-gate wait")?;
                Ok(state)
            }
            None => Ok(self
                .ready
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner())),
        }
    }
}

impl<T> ReconnectPermit<'_, T> {
    pub(super) fn finish_until(
        mut self,
        deadline: Option<AbsoluteDeadline>,
        replacement: io::Result<T>,
    ) -> io::Result<Arc<Generation<T>>> {
        let result = self.gate.finish_reconnect(deadline, replacement);
        self.active = false;
        result
    }
}

impl<T> Drop for ReconnectPermit<'_, T> {
    fn drop(&mut self) {
        if self.active {
            self.gate.abandon_reconnect();
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_with_deadline<'a, T>(
    mutex: &'a Mutex<T>,
    deadline: Option<AbsoluteDeadline>,
    stage: &str,
) -> io::Result<MutexGuard<'a, T>> {
    let Some(deadline) = deadline else {
        return Ok(lock_unpoisoned(mutex));
    };
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {
                std::thread::yield_now();
                let pause = deadline.remaining(stage)?.min(STATE_LOCK_RETRY_MAX);
                std::thread::park_timeout(pause);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_drive_task_sftp_metadata_budget_is_absolute_across_stages() {
        let started = Instant::now();
        let deadline = AbsoluteDeadline {
            expires: started + Duration::from_secs(20),
        };

        assert_eq!(
            deadline
                .remaining_at(started + Duration::from_secs(7), "reconnect")
                .unwrap(),
            Duration::from_secs(13)
        );
        assert_eq!(
            deadline
                .remaining_at(started + Duration::from_secs(19), "operation")
                .unwrap(),
            Duration::from_secs(1)
        );
        assert_eq!(
            deadline
                .remaining_at(started + Duration::from_secs(20), "operation")
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn remote_drive_task_sftp_reconnect_wait_uses_callers_remaining_budget() {
        let gate = ReconnectGate::new("old");
        let reconnect = match gate.acquire(None, |_| false).unwrap() {
            ReconnectAccess::Reconnect(reconnect) => reconnect,
            ReconnectAccess::Current(_) => panic!("stale generation must request reconnect"),
        };

        let deadline = AbsoluteDeadline::after(Duration::from_millis(10));
        let error = match gate.acquire(Some(deadline), |_| false) {
            Ok(_) => panic!("a concurrent caller must not start another reconnect"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);

        let replacement = reconnect.finish_until(None, Ok("new")).unwrap();
        assert_eq!(*replacement.value(), "new");
    }

    #[test]
    fn remote_drive_task_sftp_state_lock_wait_uses_absolute_deadline() {
        let state = Mutex::new(());
        let _held = lock_unpoisoned(&state);
        let deadline = AbsoluteDeadline::after(Duration::from_millis(10));

        let error = match lock_with_deadline(&state, Some(deadline), "scripted state lock") {
            Ok(_) => panic!("contended metadata state lock must not outlive its deadline"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn remote_drive_task_sftp_abandoned_reconnect_claim_releases_waiters() {
        let gate = ReconnectGate::new("old");
        let abandoned = match gate.acquire(None, |_| false).unwrap() {
            ReconnectAccess::Reconnect(reconnect) => reconnect,
            ReconnectAccess::Current(_) => panic!("stale generation must request reconnect"),
        };
        drop(abandoned);

        let next = match gate.acquire(None, |_| false).unwrap() {
            ReconnectAccess::Reconnect(reconnect) => reconnect,
            ReconnectAccess::Current(_) => panic!("stale generation must remain unavailable"),
        };
        let error = match next.finish_until(
            None,
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "scripted failure",
            )),
        ) {
            Ok(_) => panic!("scripted reconnect must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    }
}
