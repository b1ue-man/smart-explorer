//! Device conditions reported by an embedding host (Android: battery saver
//! and metered network). Only the Android platform adapter reads them for
//! auto-pause; desktop adapters keep their own checks and nobody sets these.

use std::sync::atomic::{AtomicU8, Ordering};

const POWER_SAVE: u8 = 0b01;
const METERED: u8 = 0b10;

/// One word so a reader never observes half of an update.
static HOST_STATE: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostState {
    pub power_save: bool,
    pub metered: bool,
}

/// Replace the host-reported conditions; the next pause check sees them.
pub fn set_host_state(state: HostState) {
    HOST_STATE.store(encode(state), Ordering::Release);
}

/// The last host-reported conditions (all false until a host reports).
pub fn host_state() -> HostState {
    decode(HOST_STATE.load(Ordering::Acquire))
}

fn encode(state: HostState) -> u8 {
    (if state.power_save { POWER_SAVE } else { 0 }) | (if state.metered { METERED } else { 0 })
}

fn decode(bits: u8) -> HostState {
    HostState {
        power_save: bits & POWER_SAVE != 0,
        metered: bits & METERED != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, HostState};

    #[test]
    fn android_task_host_state_encoding_keeps_both_conditions() {
        for power_save in [false, true] {
            for metered in [false, true] {
                let state = HostState {
                    power_save,
                    metered,
                };
                assert_eq!(decode(encode(state)), state);
            }
        }
        assert_eq!(decode(0), HostState::default());
    }
}
