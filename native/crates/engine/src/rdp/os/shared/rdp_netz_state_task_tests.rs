//! E29 (M11): the setup state of a host whose se-managed GNOME sharing is
//! switched off between sessions. The probe and the marker state are fixed
//! inputs; no file, process or network call runs in these tests.
use super::{
    recorded_state, state_with_marker, switched_off_between_sessions, OFF_BETWEEN_SESSIONS,
};
use crate::rdp::core::{
    HostKind, HostProbe, HostService, SessionMode, SetupState, RDP_PROBE_VERSION_V2,
};

fn gnome(reason: Option<&str>, managed: bool, running: bool) -> HostService {
    HostService {
        id: "gnome-console".into(),
        kind: HostKind::Gnome,
        version: None,
        session_mode: SessionMode::SharedConsole,
        port: None,
        running,
        tls: Some(true),
        nla: None,
        certificate_sha256: None,
        certificate_source: None,
        remote_apps: Vec::new(),
        evidence: Vec::new(),
        unavailable_reason: reason.map(str::to_string),
        certificate_sha1: None,
        managed_credentials: managed,
        listener_pid: None,
    }
}

fn probe(services: Vec<HostService>) -> HostProbe {
    HostProbe {
        version: RDP_PROBE_VERSION_V2,
        platform: "linux".into(),
        services,
        unavailable_reason: None,
        diagnostics: Vec::new(),
    }
}

/// A switched-off sharing that se manages, with a `Done` marker, is `Done`.
#[test]
fn rdp_netz_task_state_switched_off_sharing_with_a_done_marker_is_done() {
    let off = probe(vec![gnome(Some(OFF_BETWEEN_SESSIONS), true, true)]);
    assert_eq!(
        state_with_marker(&off, Some(SetupState::Done)),
        SetupState::Done
    );
}

/// Whatever the marker holds, a switched-off sharing is never reported as failed.
#[test]
fn rdp_netz_task_state_switched_off_sharing_is_never_failed() {
    let off = probe(vec![gnome(Some(OFF_BETWEEN_SESSIONS), true, true)]);
    let markers = [
        None,
        Some(SetupState::Done),
        Some(SetupState::NotNeeded),
        Some(SetupState::Failed("no_listener: earlier".into())),
    ];
    for marker in markers {
        let state = state_with_marker(&off, marker.clone());
        assert!(
            !matches!(state, SetupState::Failed(_)),
            "{marker:?} gives {state:?}"
        );
    }
    assert_eq!(state_with_marker(&off, None), SetupState::NotNeeded);
}

/// An enabled sharing without a listener still fails: only the switched-off
/// reason is folded into the setup state.
#[test]
fn rdp_netz_task_state_an_enabled_sharing_without_listener_still_fails() {
    let unproved = probe(vec![gnome(
        Some("The configured RDP port cannot be bound to one unambiguous process-owned listener"),
        true,
        true,
    )]);
    let state = state_with_marker(&unproved, Some(SetupState::Done));
    assert!(
        matches!(&state, SetupState::Failed(reason) if reason.starts_with("no_listener")),
        "{state:?}"
    );
}

/// A sharing that the person set up is not se's to switch off: it keeps its failure.
#[test]
fn rdp_netz_task_state_a_sharing_the_person_set_up_is_not_switched_off_by_se() {
    let own = probe(vec![gnome(Some(OFF_BETWEEN_SESSIONS), false, true)]);
    assert!(!switched_off_between_sessions(&own.services[0]));
    assert!(matches!(
        state_with_marker(&own, Some(SetupState::Done)),
        SetupState::Failed(_)
    ));
}

/// The condition needs the GNOME kind, the managed flag and the switched-off
/// reason together; the `running` flag does not decide it.
#[test]
fn rdp_netz_task_state_the_switched_off_condition_needs_all_three_facts() {
    let windows = HostService {
        kind: HostKind::WindowsWorkstation,
        ..gnome(Some(OFF_BETWEEN_SESSIONS), true, true)
    };
    assert!(!switched_off_between_sessions(&windows));
    assert!(!switched_off_between_sessions(&gnome(None, true, true)));
    assert!(switched_off_between_sessions(&gnome(
        Some(OFF_BETWEEN_SESSIONS),
        true,
        false
    )));
}

/// The recorded result of a finished setup: `Done` without a listener is a
/// failure, except for a switched-off se-managed sharing.
#[test]
fn rdp_netz_task_state_recorded_done_is_a_failure_only_without_a_listener() {
    let without = probe(Vec::new());
    assert!(matches!(
        recorded_state(&without, SetupState::Done),
        SetupState::Failed(_)
    ));
    let off = probe(vec![gnome(Some(OFF_BETWEEN_SESSIONS), true, true)]);
    assert_eq!(recorded_state(&off, SetupState::Done), SetupState::Done);
    assert_eq!(
        recorded_state(&off, SetupState::Declined),
        SetupState::Declined
    );
}
