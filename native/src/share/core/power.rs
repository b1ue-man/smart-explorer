//! Process-wide power state of the Share client.
//!
//! The Android host switches the client into low-power operation while the
//! app is not visible (`set_low_power`), asks for a connection probe after a
//! wake alarm or a network change (`request_probe`) and lets the client keep
//! the CPU awake for short work after a wake-up (`set_activity_hook`). A
//! desktop never calls any of this and stays in normal operation.

use std::sync::{Arc, OnceLock};

#[path = "power_clock.rs"]
pub(crate) mod clock;
#[path = "power_hub.rs"]
mod hub;
#[path = "power_probe.rs"]
mod probe;

#[cfg(test)]
pub(crate) use self::hub::{hold_needed, HOLD_THROTTLE_MS};
pub(crate) use self::hub::{PowerHub, PowerSubscription};
pub(crate) use self::probe::ProbeBatch;
pub use self::probe::{ProbeOutcome, ProbeTicket};

/// Hold after every server keepalive: answer, presence, relay check (K3).
pub(crate) const KEEPALIVE_HOLD_MS: u32 = 5_000;
/// Hold per connection attempt while the signal or home-relay connection
/// is missing in low-power operation.
pub(crate) const CONNECT_HOLD_MS: u32 = 15_000;
/// Hold after an incoming Direct request or decision on the signal channel.
pub(crate) const SIGNAL_ACTIVITY_HOLD_MS: u32 = 15_000;
/// Hold when an incoming peer stream starts; renewed while streams are open.
pub(crate) const STREAM_HOLD_MS: u32 = 60_000;

/// What the signal connection knows about the server's idle mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalPowerStatus {
    /// `Some(false)`: the connected server lacks `idle_keepalive_v1` and the
    /// client keeps sending heartbeats. `None` before the first connection.
    pub idle_supported: Option<bool>,
    /// The server confirmed idle mode for the current connection.
    pub idle_active: bool,
    /// Keepalive interval the server confirmed for idle mode.
    pub keepalive_secs: Option<u32>,
    /// Wall time (Unix seconds) of the last line received from the server.
    pub last_server_contact_unix: Option<i64>,
}

/// Enters (`true`) or leaves low-power operation: idle signal mode, quiet
/// peer connections closed, no periodic discovery listing.
pub fn set_low_power(on: bool) {
    global().set_low_power(on);
}

pub fn low_power() -> bool {
    global().low_power()
}

/// Asks every running signal worker to verify its server connection, after
/// a wake alarm (`false`) or a change of the default network (`true`).
pub fn request_probe(network_changed: bool) -> ProbeTicket {
    global().request_probe(network_changed)
}

/// Registers the host's wakelock hook; it receives the hold in milliseconds
/// and must return quickly (it runs on network threads).
pub fn set_activity_hook(hook: fn(hold_ms: u32)) {
    global().set_activity_hook(hook);
}

/// Requests a CPU hold from the host's hook; throttled, longer wins.
pub fn request_hold(hold_ms: u32) {
    global().request_hold(hold_ms);
}

pub fn signal_power_status() -> SignalPowerStatus {
    global().signal_status()
}

pub(crate) fn global() -> &'static Arc<PowerHub> {
    static HUB: OnceLock<Arc<PowerHub>> = OnceLock::new();
    HUB.get_or_init(|| Arc::new(PowerHub::new(Arc::new(clock::SystemClock))))
}

#[cfg(test)]
#[path = "power_tests.rs"]
mod android_background_task_power_tests;
#[cfg(test)]
#[path = "power_test_support.rs"]
pub(crate) mod test_support;
