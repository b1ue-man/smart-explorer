//! Device conditions reported by an embedding host (Android: battery saver,
//! metered network and whether scheduled jobs wait for the host's own worker
//! run). The Android platform adapter reads the first two for auto-pause and
//! the scheduling loop reads the third; desktop hosts never set any of them,
//! so they stay false there.

use std::sync::atomic::{AtomicU8, Ordering};

const POWER_SAVE: u8 = 0b001;
const METERED: u8 = 0b010;
const DEFER_SCHEDULING: u8 = 0b100;

/// One word so a reader never observes half of an update.
static HOST_STATE: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostState {
    pub power_save: bool,
    pub metered: bool,
    /// Scheduled jobs (startup, timer, real-time, on-connect) are not
    /// enqueued; running jobs, catch-up runs and auto-pause are unaffected.
    pub defer_scheduling: bool,
}

/// Replace the host-reported conditions; the next pause check sees them.
pub fn set_host_state(state: HostState) {
    HOST_STATE.store(encode(state), Ordering::Release);
}

/// Hold scheduled jobs until the host reports its conditions for the first
/// time (`set_host_state` replaces this bit). Only this bit changes, so a
/// concurrent report of the other conditions is never lost.
pub fn defer_scheduling_until_reported() {
    mark_deferred(&HOST_STATE);
}

/// The last host-reported conditions (all false until a host reports).
pub fn host_state() -> HostState {
    decode(HOST_STATE.load(Ordering::Acquire))
}

fn mark_deferred(word: &AtomicU8) {
    word.fetch_or(DEFER_SCHEDULING, Ordering::AcqRel);
}

fn encode(state: HostState) -> u8 {
    (if state.power_save { POWER_SAVE } else { 0 })
        | (if state.metered { METERED } else { 0 })
        | (if state.defer_scheduling {
            DEFER_SCHEDULING
        } else {
            0
        })
}

fn decode(bits: u8) -> HostState {
    HostState {
        power_save: bits & POWER_SAVE != 0,
        metered: bits & METERED != 0,
        defer_scheduling: bits & DEFER_SCHEDULING != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, mark_deferred, HostState};
    use std::sync::atomic::{AtomicU8, Ordering};

    #[test]
    fn android_task_host_state_encoding_keeps_both_conditions() {
        for power_save in [false, true] {
            for metered in [false, true] {
                let state = HostState {
                    power_save,
                    metered,
                    ..HostState::default()
                };
                assert_eq!(decode(encode(state)), state);
            }
        }
        assert_eq!(decode(0), HostState::default());
    }

    #[test]
    fn android_background_task_defer_bit_is_independent_of_conditions() {
        for bits in 0..8u8 {
            let state = decode(bits);
            assert_eq!(encode(state), bits);
        }
        let reported = HostState {
            power_save: true,
            metered: true,
            defer_scheduling: false,
        };
        let word = AtomicU8::new(encode(reported));
        mark_deferred(&word);
        let deferred = decode(word.load(Ordering::Acquire));
        assert!(deferred.defer_scheduling);
        assert!(deferred.power_save && deferred.metered);
        // A host report replaces the start-up deferral.
        word.store(encode(reported), Ordering::Release);
        assert!(!decode(word.load(Ordering::Acquire)).defer_scheduling);
    }
}
