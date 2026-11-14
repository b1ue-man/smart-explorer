//! L17 (E57): a sharing that se set up is switched off when a steering right ends
//! during a credential issue and no device steers any more. The steering set is
//! process-global, so the tests of this file run one at a time.
use crate::share::access_steering::{any_steering, note_steering_devices};
use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

fn devices(names: &[&str]) -> BTreeSet<(String, String)> {
    names
        .iter()
        .map(|name| (name.to_string(), format!("key-{name}")))
        .collect()
}

/// The last device that may steer this screen is withdrawn: no device steers any
/// more, so the sharing that se set up goes off.
#[test]
fn rdp_netz_task_state_no_steering_device_left_after_the_last_withdrawal() {
    let _serial = one_at_a_time();
    note_steering_devices(devices(&["device-a"]));
    assert!(any_steering());
    note_steering_devices(BTreeSet::new());
    assert!(!any_steering());
}

/// One device is withdrawn while another still steers: the sharing stays on.
#[test]
fn rdp_netz_task_state_one_steering_device_left_keeps_steering() {
    let _serial = one_at_a_time();
    note_steering_devices(devices(&["device-a", "device-b"]));
    note_steering_devices(devices(&["device-b"]));
    assert!(any_steering());
    note_steering_devices(BTreeSet::new());
}
