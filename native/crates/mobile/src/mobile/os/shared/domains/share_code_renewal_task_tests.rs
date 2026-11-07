//! E17 (plan M22; code-oberflaechen co-android-bridge, co-kontakte): the
//! `codeRenewal` member of `share.status`, the generation rule that reads the
//! identity again, and the keys of `share.status` a renewal must leave alone.
//! The module sits in `share_status::code_renewal`, so it reaches the
//! `pub(super)` items of `share_status` through its parents.
use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use super::super::{status_json, StatusInput, WorkerFacts};
use super::value;
use crate::daemon::CodeRenewalView;
use crate::share::discovery_state::DiscoveryUiState;
use crate::share::{ExecProviderStatus, ShareProfiles};

/// Input E of code-oberflaechen §4: 2 devices pending, 1 of them on an older version.
fn view_e() -> CodeRenewalView {
    CodeRenewalView {
        active: true,
        generation: 3,
        pending_devices: 2,
        legacy_devices: 1,
        pending_names: vec!["P".into(), "Q".into()],
        since: Some(100),
        deferred: false,
        error: None,
    }
}

fn view_of_generation(generation: u64) -> CodeRenewalView {
    CodeRenewalView {
        active: true,
        generation,
        pending_devices: 1,
        ..Default::default()
    }
}

fn exec_provider() -> ExecProviderStatus {
    ExecProviderStatus {
        available: true,
        provider: "android-subreaper".into(),
        detail: String::new(),
        elevated: false,
        user_label: "Android-App".into(),
    }
}

fn profiles_with_contact() -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts.push(
        serde_json::from_value(json!({
            "id": "c1",
            "display_name": "Laptop",
            "lookup_id": "l1",
            "expected_fingerprint": "fp",
            "auto_connect": true,
            "auto_open": false,
            "status": "Available",
        }))
        .expect("contact"),
    );
    profiles
}

/// `share.status` for the given worker facts and profiles, built as the bridge builds it.
fn status_of(worker: Option<&WorkerFacts>, profiles: &ShareProfiles) -> Value {
    let discovery = DiscoveryUiState::default();
    let exec = exec_provider();
    let notices = VecDeque::new();
    let aliases = BTreeMap::new();
    status_json(&StatusInput {
        worker,
        profiles,
        identity: None,
        server: "",
        poll_error: None,
        discovery: &discovery,
        offer_aliases: &aliases,
        last_exchange: None,
        notices: &notices,
        now_secs: 1_700_000_000,
        exec_provider: &exec,
    })
}

#[test]
fn code_renewal_surface_task_bridge_code_renewal_names_summary_and_since_of_e() {
    let facts = WorkerFacts {
        code_renewal: Some(view_e()),
        ..Default::default()
    };
    let status = status_of(Some(&facts), &ShareProfiles::default());
    let renewal = &status["codeRenewal"];
    assert_eq!(
        renewal["summary"],
        "Code wird erneuert, 2 Geräte stehen aus (davon 1 mit älterer Version)"
    );
    assert_eq!(renewal["namesLine"], "Ausstehend: P, Q");
    assert_eq!(renewal["pendingMore"], 0);
    assert_eq!(renewal["sinceMs"], 100_000);
    assert_eq!(status["codeRenewal"], value(Some(&view_e())));
}

#[test]
fn code_renewal_surface_task_bridge_code_renewal_is_null_without_a_view() {
    let status = status_of(None, &ShareProfiles::default());
    assert_eq!(status["codeRenewal"], Value::Null);
    let facts = WorkerFacts::default();
    let running = status_of(Some(&facts), &ShareProfiles::default());
    assert_eq!(running["codeRenewal"], Value::Null);
    assert_eq!(value(None), Value::Null);
}

#[test]
fn code_renewal_surface_task_bridge_renewal_changed_follows_the_generation() {
    let facts_of = |generation: u64| WorkerFacts {
        code_renewal: Some(view_of_generation(generation)),
        ..Default::default()
    };
    // Generation 2 to 3: the identity is read again.
    assert!(facts_of(3).renewal_changed(Some(&facts_of(2))));
    // Generation 3 to 3: nothing changed.
    assert!(!facts_of(3).renewal_changed(Some(&facts_of(3))));
    // No renewal before and none now.
    assert!(!WorkerFacts::default().renewal_changed(None));
    // No renewal before, one now.
    assert!(facts_of(3).renewal_changed(None));
}

#[test]
fn code_renewal_surface_task_bridge_status_keeps_its_keys_and_devices_with_a_renewal() {
    let profiles = profiles_with_contact();
    let plain = WorkerFacts {
        running: true,
        ..Default::default()
    };
    let renewing = WorkerFacts {
        code_renewal: Some(view_e()),
        ..plain.clone()
    };
    let before = status_of(Some(&plain), &profiles);
    let after = status_of(Some(&renewing), &profiles);

    let object = before.as_object().expect("share.status is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    // The 21 keys of `share.status` before the batch (`git show HEAD:` share_status.rs) plus `codeRenewal`.
    let mut expected = vec![
        "autoHomeMigrations",
        "codeRenewal",
        "connectionExports",
        "devices",
        "discovery",
        "execProvider",
        "execTargets",
        "exports",
        "identity",
        "incoming",
        "lanPresence",
        "lastError",
        "notices",
        "outgoing",
        "relayUrl",
        "rdpSetup",
        "removedDevices",
        "rooms",
        "running",
        "server",
        "connected",
        "writeGrants",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(before["devices"][0]["contactId"], "c1");
    assert_eq!(before["codeRenewal"], Value::Null);

    for (key, value_before) in object {
        if key.as_str() != "codeRenewal" {
            assert_eq!(
                &after[key.as_str()],
                value_before,
                "{key} changed by the renewal"
            );
        }
    }
}
