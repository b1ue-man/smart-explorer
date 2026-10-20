//! The time limit of the session probes (U34-2): a service that does not
//! answer yet at login (D-Bus, the X11 or Wayland server) must not hold the
//! shell back. Every probe runs in a thread of its own and is waited for at
//! most [`PROBE_WAIT`]; running out of time counts as "not reachable". A probe
//! that the shell repeats every second ([`timed_gated`]) is not started again
//! while its previous run still hangs, so a dead service leaves one thread,
//! not one per round.
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// How long a probe is waited for.
pub(crate) const PROBE_WAIT: Duration = Duration::from_secs(2);

/// Names of the gated probes whose last run has not ended yet.
static RUNNING: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn running() -> std::sync::MutexGuard<'static, Vec<&'static str>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `work` in a thread of its own; `None` when it does not end within
/// [`PROBE_WAIT`] or the thread cannot start.
pub(crate) fn timed<T, F>(name: &str, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("se-probe-{name}"))
        .spawn(move || {
            let _ = sender.send(work());
        })
        .ok()?;
    receiver.recv_timeout(PROBE_WAIT).ok()
}

/// Like [`timed`], but `None` at once while the previous run of the probe
/// `name` is still going.
pub(crate) fn timed_gated<T, F>(name: &'static str, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    {
        let mut busy = running();
        if busy.contains(&name) {
            return None;
        }
        busy.push(name);
    }
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("se-probe-{name}"))
        .spawn(move || {
            let value = work();
            running().retain(|known| *known != name);
            let _ = sender.send(value);
        });
    if spawned.is_err() {
        running().retain(|known| *known != name);
        return None;
    }
    receiver.recv_timeout(PROBE_WAIT).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_that_answers_returns_its_value() {
        assert_eq!(timed("answer", || 7), Some(7));
        assert_eq!(timed_gated("answer-gated", || 8), Some(8));
    }

    #[test]
    fn a_gated_probe_is_not_started_while_it_hangs() {
        let started = timed_gated("hang-gated", || {
            std::thread::sleep(PROBE_WAIT + Duration::from_millis(300));
            1
        });
        assert_eq!(started, None, "the hanging probe times out");
        assert_eq!(timed_gated("hang-gated", || 2), None, "still running");
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            timed_gated("hang-gated", || 3),
            Some(3),
            "ended, free again"
        );
    }
}
