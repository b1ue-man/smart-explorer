//! E17 (plan M22; code-oberflaechen co-cli-json, co-cli-text, co-kontakte): the
//! renewal member and text lines of `se share status`, and the keys of the
//! status document that a renewal must leave unchanged. This module sits in
//! `status::code_renewal_output`, so it reaches the private `json_value` of
//! `status` through its parent.
use super::super::json_value;
use super::{text_lines, value};
use crate::daemon::{CodeRenewalView, ShareWorkerSnapshot};
use crate::share::ShareProfiles;
use serde_json::{json, Value};

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

/// Input E2 of code-oberflaechen §4: 40 devices pending, 32 of them named.
/// Legacy count 0 is an assumption of this test (the expectation does not name it).
fn view_e2() -> CodeRenewalView {
    CodeRenewalView {
        active: true,
        generation: 3,
        pending_devices: 40,
        legacy_devices: 0,
        pending_names: (1..=32).map(|number| format!("G{number:02}")).collect(),
        since: Some(100),
        deferred: false,
        error: None,
    }
}

#[test]
fn code_renewal_surface_task_json_member_carries_every_field_of_e() {
    assert_eq!(
        value(Some(&view_e())),
        json!({
            "active": true,
            "generation": 3,
            "pending_devices": 2,
            "legacy_devices": 1,
            "pending_names": ["P", "Q"],
            "pending_more": 0,
            "since": 100,
            "deferred": false,
            "error": null,
            "summary": "Direct code is being renewed, 2 devices pending (of which 1 on an older version)",
        })
    );
}

#[test]
fn code_renewal_surface_task_json_member_is_null_without_a_renewal() {
    assert_eq!(value(None), Value::Null);
}

#[test]
fn code_renewal_surface_task_json_counts_the_devices_beyond_the_named() {
    let member = value(Some(&view_e2()));
    assert_eq!(member["pending_devices"], 40);
    assert_eq!(member["pending_names"].as_array().map(Vec::len), Some(32));
    assert_eq!(member["pending_more"], 8);
}

#[test]
fn code_renewal_surface_task_text_lines_of_e_list_every_pending_name() {
    assert_eq!(
        text_lines(Some(&view_e())),
        vec![
            "code_renewal\tactive\tpending=2\tlegacy=1\tgeneration=3\tDirect code is being renewed, 2 devices pending (of which 1 on an older version)".to_string(),
            "code_renewal_pending\tP".to_string(),
            "code_renewal_pending\tQ".to_string(),
        ]
    );
}

#[test]
fn code_renewal_surface_task_text_lines_are_empty_without_a_renewal() {
    assert!(text_lines(None).is_empty());
}

#[test]
fn code_renewal_surface_task_text_lines_name_32_devices_and_count_the_rest() {
    let lines = text_lines(Some(&view_e2()));
    assert_eq!(lines.len(), 34);
    assert_eq!(
        lines[0],
        "code_renewal\tactive\tpending=40\tlegacy=0\tgeneration=3\tDirect code is being renewed, 40 devices pending"
    );
    assert_eq!(lines[1], "code_renewal_pending\tG01");
    assert_eq!(lines[32], "code_renewal_pending\tG32");
    assert_eq!(lines[33], "code_renewal_more\t8");
}

#[test]
fn code_renewal_surface_task_text_lines_flatten_names_and_report_deferred_and_error() {
    let view = CodeRenewalView {
        active: true,
        generation: 4,
        pending_devices: 1,
        legacy_devices: 0,
        pending_names: vec!["a\tb".into()],
        since: None,
        deferred: true,
        error: Some("Bad\r\nline".into()),
    };
    assert_eq!(
        text_lines(Some(&view)),
        vec![
            "code_renewal\tactive\tpending=1\tlegacy=0\tgeneration=4\tDirect code is being renewed, 1 device pending".to_string(),
            "code_renewal_pending\ta b".to_string(),
            "code_renewal_deferred\tAnother renewal waits until a running transition ends".to_string(),
            "code_renewal_error\tBad  line".to_string(),
        ]
    );
}

#[test]
fn code_renewal_surface_task_status_document_keeps_its_keys_and_contacts() {
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
    let mut snapshot = ShareWorkerSnapshot {
        running: true,
        relay_url: "https://relay.example".into(),
        ..Default::default()
    };
    let without = json_value(&snapshot, None, &profiles);
    let object = without.as_object().expect("status document is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    // The 20 keys of the document before the batch (`git show HEAD:` status.rs) plus
    // `code_renewal` and `discoverable_renewals` (E49).
    let mut expected = vec![
        "access_gaps",
        "candidates",
        "code_renewal",
        "code_rights",
        "connected",
        "contacts",
        "discoverable",
        "discoverable_renewals",
        "events",
        "grants",
        "last_error",
        "legacy_requests",
        "pending_requests",
        "rdp_setup",
        "relay_url",
        "requests",
        "rights_pending",
        "route_measurement_note",
        "routes",
        "rooms",
        "running",
        "worker",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(without["code_renewal"], Value::Null);
    assert_eq!(without["running"], true);
    assert_eq!(without["relay_url"], "https://relay.example");
    assert_eq!(without["contacts"][0]["id"], "c1");
    assert_eq!(without["contacts"][0]["name"], "Laptop");
    let mut contact_keys: Vec<&str> = without["contacts"][0]
        .as_object()
        .expect("contact is an object")
        .keys()
        .map(String::as_str)
        .collect();
    contact_keys.sort_unstable();
    assert_eq!(
        contact_keys,
        [
            "access",
            "authorization",
            "connectivity",
            "fingerprint",
            "id",
            "name",
            "status",
        ]
    );

    snapshot.code_renewal = Some(view_e());
    let with = json_value(&snapshot, None, &profiles);
    assert_eq!(with["code_renewal"], value(Some(&view_e())));
    for (key, before) in object {
        if key.as_str() != "code_renewal" {
            assert_eq!(&with[key.as_str()], before, "{key} changed by the renewal");
        }
    }
}
