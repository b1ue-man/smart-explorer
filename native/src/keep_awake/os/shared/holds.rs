//! Counted holds and the worker thread that applies them (RV1, contract V4).
//! `hold` and dropping a hold only update the counters and nudge the worker,
//! so both are cheap and safe on any thread, including async runtime threads
//! (the Linux backend must not run its D-Bus calls there). The worker starts
//! with the first hold, brings the platform backend in line with the
//! counters after every change (and when the backend asks for renewals), and
//! ends, releasing everything, once no hold is left.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

use super::platform::Backend;
use super::types::{Applied, KeepAwakeStatus, Reason};

const REASONS: usize = Reason::ALL.len();
/// Without renewals the worker only waits for changes; the wait is bounded
/// so a lost nudge cannot leave the backend stale for long.
const IDLE_WAIT: Duration = Duration::from_secs(600);

struct Shared {
    counts: [usize; REASONS],
    /// Nudges the running worker; `None` while no worker runs.
    worker: Option<Sender<()>>,
    applied: Applied,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    counts: [0; REASONS],
    worker: None,
    applied: Applied::NONE,
});

fn shared() -> MutexGuard<'static, Shared> {
    // Every update leaves the counters consistent; a panic elsewhere must not
    // take them down with it.
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the system awake while it lives. Holds are counted per reason, so
/// nested and parallel holders share one operating-system request; dropping
/// the last hold of a reason releases it. User-initiated sleep (lid, power
/// button) is never prevented.
#[must_use = "the system may sleep as soon as the hold is dropped"]
#[derive(Debug)]
pub struct KeepAwake {
    reason: Reason,
}

impl KeepAwake {
    pub fn reason(&self) -> Reason {
        self.reason
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        let mut shared = shared();
        let count = &mut shared.counts[self.reason.index()];
        *count = count.saturating_sub(1);
        if let Some(worker) = &shared.worker {
            let _ = worker.try_send(());
        }
    }
}

/// Takes a hold for `reason`; never fails. Problems of the operating-system
/// request are reported by `status().unavailable`. Bind the hold to a named
/// variable (`let _awake = hold(..)`): `let _ = hold(..)` drops it at once.
pub fn hold(reason: Reason) -> KeepAwake {
    let mut shared = shared();
    let count = &mut shared.counts[reason.index()];
    *count = count.saturating_add(1);
    let nudged = shared
        .worker
        .as_ref()
        .is_some_and(|worker| worker.try_send(()).is_ok());
    if !nudged {
        spawn_worker(&mut shared);
    }
    KeepAwake { reason }
}

/// The live holds and the state of the operating-system request.
pub fn status() -> KeepAwakeStatus {
    let shared = shared();
    KeepAwakeStatus {
        holds: Reason::ALL
            .iter()
            .filter_map(|reason| {
                let count = shared.counts[reason.index()];
                (count > 0).then_some((*reason, count))
            })
            .collect(),
        engaged: shared.applied.engaged,
        throttling_off: shared.applied.throttling_off,
        unavailable: shared.applied.unavailable.clone(),
    }
}

fn spawn_worker(shared: &mut Shared) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    match std::thread::Builder::new()
        .name("keep-awake".into())
        .spawn(move || run_worker(receiver))
    {
        Ok(_) => shared.worker = Some(sender),
        Err(error) => {
            shared.applied.unavailable =
                Some(format!("Wachhalten: Hilfs-Thread nicht startbar: {error}"));
        }
    }
}

fn held(counts: &[usize; REASONS]) -> [bool; REASONS] {
    let mut held = [false; REASONS];
    for reason in Reason::ALL {
        held[reason.index()] = counts[reason.index()] > 0;
    }
    held
}

fn run_worker(changes: Receiver<()>) {
    let mut backend = Backend::new();
    let wake = backend.wake_receiver();
    loop {
        let wanted = held(&shared().counts);
        let applied = backend.apply(wanted);
        {
            let mut shared = shared();
            shared.applied = applied;
            if shared.counts.iter().all(|count| *count == 0) {
                if wanted.iter().any(|held| *held) {
                    continue;
                }
                // The backend released everything for the empty counters; a
                // later hold starts a new worker.
                shared.worker = None;
                return;
            }
        }
        wait(
            &changes,
            wake.as_ref(),
            backend.renew_after().unwrap_or(IDLE_WAIT),
        );
    }
}

fn wait(changes: &Receiver<()>, wake: Option<&Receiver<()>>, timeout: Duration) {
    let never = crossbeam_channel::never::<()>();
    let wake = wake.unwrap_or(&never);
    crossbeam_channel::select! {
        recv(changes) -> _ => {},
        recv(wake) -> _ => {},
        default(timeout) => {},
    }
    while changes.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_keep_awake_counts_nested_holds_per_reason() {
        let outer = hold(Reason::RemoteTask);
        let inner = hold(Reason::RemoteTask);
        let other = hold(Reason::PeerService);
        let counted = |reason| {
            status()
                .holds
                .iter()
                .find(|(held, _)| *held == reason)
                .map(|(_, count)| *count)
                .unwrap_or(0)
        };
        assert!(counted(Reason::RemoteTask) >= 2);
        assert!(counted(Reason::PeerService) >= 1);
        drop(inner);
        assert!(counted(Reason::RemoteTask) >= 1);
        drop(outer);
        drop(other);
        assert_eq!(
            held(&[0, 2, 0]),
            [false, true, false],
            "only reasons with holds are requested"
        );
    }
}
