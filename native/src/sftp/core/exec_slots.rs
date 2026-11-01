//! The digest commands one SSH connection runs at once (digest_exec.rs). A
//! remote snapshot walk lists many folders together, and in checksum mode
//! each of them asks for its MD5s on a session channel of its own; the server
//! opens only so many per connection (OpenSSH `MaxSessions`, default 10, of
//! which the SFTP session holds one). A channel refused while other digest
//! commands run is a full connection, not a server without exec: the asker
//! waits until one of them ends and tries again, and the limit drops to what
//! ran at the refusal. Only a refusal while no other digest command runs
//! means this connection gives none (`Ok(None)` of `digest_exec::run`), so
//! the caller takes its own way (the content) only where the server leaves
//! no other.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Digest commands at once before the server refused one: `MaxSessions` 10
/// less the SFTP session and one channel for the agent's exec, a terminal or
/// the rename channel.
const START_LIMIT: usize = 8;
/// How often a waiting asker looks at its cancel flag.
const POLL: Duration = Duration::from_millis(200);

pub(super) struct ExecSlots {
    state: Mutex<State>,
    freed: Condvar,
}

struct State {
    running: usize,
    limit: usize,
}

impl Default for ExecSlots {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                running: 0,
                limit: START_LIMIT,
            }),
            freed: Condvar::new(),
        }
    }
}

/// One running digest command; its end frees the slot.
pub(super) struct Slot<'a>(&'a ExecSlots);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.running = state.running.saturating_sub(1);
        drop(state);
        self.0.freed.notify_all();
    }
}

impl ExecSlots {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A slot once fewer commands run than the limit; `None` once `cancel`
    /// is set.
    pub(super) fn enter(&self, cancel: &AtomicBool) -> Option<Slot<'_>> {
        let mut state = self.lock();
        while state.running >= state.limit {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            state = match self.freed.wait_timeout(state, POLL) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        state.running += 1;
        Some(Slot(self))
    }

    /// The server refused a channel to the holder of `slot`. `true`: other
    /// digest commands run, the limit drops to them, and the caller gives
    /// its slot back and enters again (it waits until one of them ends).
    /// `false`: none runs, this connection gives no channel now.
    pub(super) fn refused(&self, slot: &Slot<'_>) -> bool {
        debug_assert!(std::ptr::eq(slot.0, self));
        let mut state = self.lock();
        let others = state.running.saturating_sub(1);
        if others == 0 {
            return false;
        }
        state.limit = state.limit.min(others);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecSlots, START_LIMIT};
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// A full connection makes askers wait, never give up: a refusal while
    /// others run lowers the limit to them, and the asker runs once one of
    /// them ended; at most the limit run at once.
    #[test]
    fn abgleich_task_exec_slots_wait_for_a_full_connection_instead_of_giving_up() {
        let slots = ExecSlots::default();
        let cancel = AtomicBool::new(false);
        let held: Vec<_> = (0..START_LIMIT)
            .map(|_| slots.enter(&cancel).expect("free slot"))
            .collect();
        assert!(slots.refused(&held[0]), "others run: wait and try again");
        assert_eq!(slots.lock().limit, START_LIMIT - 1);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let slot = slots.enter(&cancel).expect("a slot after one ended");
                let state = slots.lock();
                let seen = (state.running, state.limit);
                drop(state);
                drop(slot);
                seen
            });
            std::thread::sleep(Duration::from_millis(50));
            assert!(!waiter.is_finished(), "the limit is reached: it waits");
            let mut held = held;
            held.pop();
            held.pop();
            let (running, limit) = waiter.join().expect("waiter");
            assert_eq!(limit, START_LIMIT - 1);
            assert!(running <= limit, "{running} running, limit {limit}");
        });
    }

    /// A refusal while nothing else runs means the connection gives no
    /// channel; a canceled asker stops waiting.
    #[test]
    fn abgleich_task_exec_slots_alone_refused_gives_up_and_cancel_ends_the_wait() {
        let slots = ExecSlots::default();
        let cancel = AtomicBool::new(false);
        let alone = slots.enter(&cancel).expect("free slot");
        assert!(!slots.refused(&alone), "nothing else runs");
        assert_eq!(slots.lock().limit, START_LIMIT);
        let held: Vec<_> = (1..START_LIMIT)
            .map(|_| slots.enter(&cancel).expect("free slot"))
            .collect();
        let canceled = AtomicBool::new(true);
        assert!(slots.enter(&canceled).is_none(), "full and canceled");
        drop(held);
        drop(alone);
        assert_eq!(slots.lock().running, 0);
    }
}
