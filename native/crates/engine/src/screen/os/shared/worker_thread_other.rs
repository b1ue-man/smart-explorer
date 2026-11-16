//! Threads of the screen host process off Windows: they keep their
//! priority (raising it needs `CAP_SYS_NICE` on Linux) and the tick sleeps.

use std::time::Instant;

pub fn raise_priority() -> bool {
    false
}

#[derive(Default)]
pub struct PreciseWait;

impl PreciseWait {
    pub fn until(&self, due: Instant) {
        let wait = due.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}
