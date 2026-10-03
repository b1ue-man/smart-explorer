//! Android backend of the wake holds (RV1, V4): the existing CPU-hold hook of
//! the Share client (`share::power::request_hold`), which the app turns into
//! a partial wake lock with a timeout (`WakeKeeper`). A hold of `HOLD_MS` is
//! renewed every `RENEW` while holds live, and at once when the app enters
//! low-power operation: the backend subscribes to the power hub, which wakes
//! its subscribers on that change. In the foreground the screen keeps the CPU
//! awake and the hook stays quiet.

use std::time::Duration;

use crossbeam_channel::Receiver;

use super::types::{Applied, Reason};
use crate::share::power::PowerSubscription;

/// Longest hold the app grants one request (`WakeKeeper.MAX_HOLD_MS` is
/// twice this), renewed at half of it.
const HOLD_MS: u32 = 60_000;
const RENEW: Duration = Duration::from_secs(30);

pub(super) struct Backend {
    subscription: PowerSubscription,
    wake: Receiver<()>,
}

impl Backend {
    pub(super) fn new() -> Self {
        let (sender, wake) = crossbeam_channel::bounded(1);
        let subscription = crate::share::power::global().subscribe(sender);
        Self { subscription, wake }
    }

    pub(super) fn apply(&mut self, held: [bool; Reason::ALL.len()]) -> Applied {
        // The hub hands every subscriber a share of a connection probe; this
        // subscriber is no signal worker, so it answers at once (an empty
        // answer, the workers' answers decide the outcome) and a probe never
        // waits for it.
        drop(self.subscription.take_probe());
        let engaged = held.iter().any(|held| *held);
        if engaged {
            crate::share::power::request_hold(HOLD_MS);
        }
        Applied {
            engaged,
            throttling_off: false,
            unavailable: None,
        }
    }

    pub(super) fn renew_after(&self) -> Option<Duration> {
        Some(RENEW)
    }

    pub(super) fn wake_receiver(&self) -> Option<Receiver<()>> {
        Some(self.wake.clone())
    }
}
