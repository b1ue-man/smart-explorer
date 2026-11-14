//! E42 (L1): „Bildschirm steuern: Erlauben“ hands exactly the steering
//! devices to the login self-setup, each with the end of its right. The
//! runtime reads the same relation list for the steering devices and for
//! these grants (`relation_access`); `note_steering` merges a device named
//! twice (`one_per_device`).
use super::login_grants;
use crate::login::core::self_grants::{one_per_device, SteeringGrant};
use crate::share::access_grant::{AccessGrant, Capability, CapabilityDecision};

const NOW: i64 = 1_800_000_000;
const ALLOW: CapabilityDecision = CapabilityDecision::Allow;

fn key(byte: u8) -> String {
    iroh::SecretKey::from_bytes(&[byte; 32])
        .public()
        .to_string()
}

/// One relation of `device` whose screen right is `screen` until `end`.
fn relation<'a>(
    device: &'a str,
    public_key: &'a str,
    screen: CapabilityDecision,
    end: Option<i64>,
) -> (&'a str, &'a str, AccessGrant) {
    let mut grant = AccessGrant::default().with(Capability::ScreenControl, screen);
    grant.valid_until = end;
    (device, public_key, grant)
}

fn grant(device: &str, public_key: &str, valid_until: Option<u64>) -> SteeringGrant {
    SteeringGrant {
        peer_id: device.into(),
        public_key: public_key.into(),
        valid_until,
    }
}

#[test]
fn login_selbst_task_steering_right_gives_exactly_one_grant_for_that_device() {
    let (viewer, files) = (key(1), key(2));
    let end = NOW + 3600;
    let (name, public_key, mut commands) =
        relation("files-only", &files, CapabilityDecision::Off, None);
    // Commands without steering give no login grant.
    commands.set(Capability::ExecCommands, ALLOW);
    let rights = vec![
        relation("viewer", &viewer, ALLOW, Some(end)),
        (name, public_key, commands),
    ];
    let grants = one_per_device(login_grants(rights, NOW));
    assert_eq!(grants, vec![grant("viewer", &viewer, Some(end as u64))]);
}

#[test]
fn login_selbst_task_revocation_empties_the_list() {
    let viewer = key(3);
    let granted = login_grants([relation("viewer", &viewer, ALLOW, None)], NOW);
    assert_eq!(granted, vec![grant("viewer", &viewer, None)]);
    // Revoked (off) or only asking: no login grant remains.
    for revoked in [CapabilityDecision::Off, CapabilityDecision::Ask] {
        let rights = [relation("viewer", &viewer, revoked, None)];
        assert!(login_grants(rights, NOW).is_empty(), "{revoked:?}");
    }
    let none: Vec<(&str, &str, AccessGrant)> = Vec::new();
    assert!(login_grants(none, NOW).is_empty());
}

#[test]
fn login_selbst_task_past_end_is_not_listed() {
    let viewer = key(4);
    let at =
        |end: Option<i64>, now: i64| login_grants([relation("viewer", &viewer, ALLOW, end)], now);
    // The end is inclusive, like `AccessGrant::expired_at`.
    assert!(at(Some(NOW - 1), NOW).is_empty());
    assert!(at(Some(NOW), NOW).is_empty());
    let next = Some((NOW + 1) as u64);
    assert_eq!(at(Some(NOW + 1), NOW), vec![grant("viewer", &viewer, next)]);
    // An end of at most 0 seconds becomes no end for the login service.
    assert_eq!(at(Some(0), -10), vec![grant("viewer", &viewer, None)]);
}

#[test]
fn login_selbst_task_direct_and_room_of_one_device_give_one_entry() {
    let viewer = key(5);
    let (direct, room) = (NOW + 60, NOW + 7200);
    let rights = vec![
        relation("viewer", &viewer, ALLOW, Some(direct)),
        relation("viewer", &viewer, ALLOW, Some(room)),
    ];
    let listed = login_grants(rights, NOW);
    assert_eq!(listed.len(), 2, "one per relation before the merge");
    let merged = one_per_device(listed);
    assert_eq!(merged, vec![grant("viewer", &viewer, Some(room as u64))]);
    // A relation without end outlasts every end.
    let open = vec![
        relation("viewer", &viewer, ALLOW, Some(room)),
        relation("viewer", &viewer, ALLOW, None),
    ];
    let merged = one_per_device(login_grants(open, NOW));
    assert_eq!(merged, vec![grant("viewer", &viewer, None)]);
}
