//! Share low-power facade (api.md §3, §4.1, §5): app visibility switches the
//! Share client's low-power mode, `share.wake` runs a connection probe for
//! the wake alarm or a network change, short CPU holds requested by the
//! Share client become `wake` events for the host's wake lock, and
//! `ShareStatus.power` reports the server's idle support.
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use super::args::opt_bool;
use crate::mobile::{ApiError, Runtime};
use crate::share::power::{self, SignalPowerStatus};

/// Visibility last applied to the Share client (0 = never reported).
static APPLIED_VISIBILITY: AtomicU8 = AtomicU8::new(0);
const VISIBLE: u8 = 1;
const HIDDEN: u8 = 2;

/// Registers the hold hook before the embedded worker can start the Share
/// client, so no hold request of the first service is lost.
pub(crate) fn install_activity_hook() {
    power::set_activity_hook(on_activity);
}

/// Called by the Share client (throttled there): the host keeps the CPU
/// awake for `hold_ms` (event `wake`) and the poller drains at once, so an
/// incoming request reaches the notification without the background delay.
fn on_activity(hold_ms: u32) {
    if let Some(runtime) = Runtime::installed() {
        runtime.emit(wake_event(hold_ms));
    }
    super::share_state::wake_for_activity(Duration::from_millis(u64::from(hold_ms)));
}

fn wake_event(hold_ms: u32) -> Value {
    json!({ "type": "wake", "ms": hold_ms })
}

/// `sys.hostState.foreground`: a hidden app puts the Share client into its
/// low-power mode. Repeated reports of the same visibility change nothing.
pub(super) fn apply_visibility(foreground: bool) {
    let visibility = if foreground { VISIBLE } else { HIDDEN };
    if APPLIED_VISIBILITY.swap(visibility, Ordering::AcqRel) != visibility {
        power::set_low_power(!foreground);
    }
}

/// `share.wake {networkChanged}` → `{ok, reconnected}`: waits for the probe
/// (the Share client bounds it to 12 s). A worker that ended is started
/// again without waiting; its new service connects by itself.
pub(super) fn wake(args: &Value) -> Result<Value, ApiError> {
    let network_changed = opt_bool(args, "networkChanged", false);
    if let Err(error) = crate::daemon::ensure_embedded_daemon(Duration::ZERO) {
        if let Some(runtime) = Runtime::installed() {
            runtime.record_error("Share-Verbindung prüfen", &error);
        }
    }
    let outcome = power::request_probe(network_changed).wait();
    super::share_state::wake();
    Ok(json!({ "ok": outcome.ok, "reconnected": outcome.reconnected }))
}

/// `ShareStatus.power`.
pub(super) fn power_json(status: &SignalPowerStatus) -> Value {
    json!({
        "idleSupported": status.idle_supported,
        "idleActive": status.idle_active,
        "keepaliveSecs": status.keepalive_secs,
        "lastServerContactMs": status
            .last_server_contact_unix
            .map(|seconds| seconds.saturating_mul(1000)),
    })
}

/// The current `ShareStatus.power`.
pub(super) fn current_power_json() -> Value {
    power_json(&power::signal_power_status())
}

/// Drops the server contact time before the poller compares statuses: it
/// moves with every keepalive and alone must not wake the app with a
/// `share` event (the watched Share page keeps it for a live display).
pub(super) fn strip_volatile(status: &mut Value) {
    if let Some(power) = status.get_mut("power").and_then(Value::as_object_mut) {
        power.remove("lastServerContactMs");
    }
}

#[cfg(test)]
mod tests {
    use super::{power_json, strip_volatile, wake_event, SignalPowerStatus};
    use serde_json::{json, Value};

    #[test]
    fn android_background_task_power_status_json_shape() {
        let unknown = power_json(&SignalPowerStatus {
            idle_supported: None,
            idle_active: false,
            keepalive_secs: None,
            last_server_contact_unix: None,
        });
        assert_eq!(
            unknown,
            json!({
                "idleSupported": Value::Null,
                "idleActive": false,
                "keepaliveSecs": Value::Null,
                "lastServerContactMs": Value::Null,
            })
        );
        let idle = power_json(&SignalPowerStatus {
            idle_supported: Some(true),
            idle_active: true,
            keepalive_secs: Some(180),
            last_server_contact_unix: Some(1_700_000_000),
        });
        assert_eq!(idle["idleSupported"], true);
        assert_eq!(idle["idleActive"], true);
        assert_eq!(idle["keepaliveSecs"], 180);
        assert_eq!(idle["lastServerContactMs"], 1_700_000_000_000i64);
    }

    #[test]
    fn android_background_task_contact_time_alone_is_no_status_change() {
        let status = |contact: i64| {
            let mut value = json!({
                "running": true,
                "power": power_json(&SignalPowerStatus {
                    idle_supported: Some(true),
                    idle_active: true,
                    keepalive_secs: Some(180),
                    last_server_contact_unix: Some(contact),
                }),
            });
            strip_volatile(&mut value);
            value.to_string()
        };
        assert_eq!(status(1_700_000_000), status(1_700_000_180));
        let mut other = json!({ "running": true });
        strip_volatile(&mut other);
        assert_eq!(other, json!({ "running": true }));
    }

    #[test]
    fn android_background_task_wake_event_carries_hold() {
        assert_eq!(wake_event(15_000), json!({ "type": "wake", "ms": 15_000 }));
    }
}
