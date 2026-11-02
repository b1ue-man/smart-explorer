//! G4 (review 2026-10-31): the bridge refuses rights fields of the wrong
//! type or with an unknown name as `invalid` instead of passing over them,
//! so no caller gets more rights than it asked for; the forms the Kotlin
//! app sends keep working.
use super::*;

#[test]
fn abgleich_task_bridge_refuses_wrongly_typed_rights_fields() {
    let refused = [
        // The example of the review: a text `write` would keep the preset's write right.
        json!({"preset": "read-write", "rights": {"write": "false"}}),
        json!({"preset": "read-write", "rights": {"write": null}}),
        json!({"rights": {"folders": {"kind": "selected", "paths": ["/a", 1]}}}),
        json!({"rights": {"folders": {"kind": "selected", "paths": "/a"}}}),
        json!({"rights": {"folders": {"kind": "selected", "paths": ["/a"], "connections": "yes"}}}),
        json!({"rights": {"folders": {"kind": "all", "paths": ["/a"]}}}),
        json!({"rights": {"folders": {"kind": "selected", "path": ["/a"]}}}),
        json!({"rights": {"folders": "all"}}),
        json!({"rights": {"folders": null}}),
        json!({"rights": {"commands": true}}),
        json!({"rights": {"screenView": null}}),
        json!({"rights": {"account": 1}}),
        json!({"rights": {"validUntil": "tomorrow"}}),
        // A misspelt field would leave the preset's value in place.
        json!({"preset": "all", "rights": {"screen_control": "off"}}),
        json!({"rights": "all"}),
        json!({"rights": ["write"]}),
        json!({"preset": 5}),
        json!({"preset": "everything"}),
    ];
    for args in refused {
        let error = rights_arg(&args, None).expect_err(&args.to_string());
        assert_eq!(error.kind, "invalid", "{args}: {}", error.message);
    }
}

#[test]
fn abgleich_task_bridge_reads_the_rights_the_app_sends() {
    let read_only = rights_arg(
        &json!({"preset": "read-write", "rights": {"write": false}}),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(!read_only.write);
    assert_eq!(read_only.folders, FolderChoice::All);

    let selected = rights_arg(
        &json!({"rights": {
            "folders": {"kind": "selected", "paths": ["/home/u/Docs"], "connections": false},
            "write": true, "commands": "ask", "terminal": "off", "screenView": "allow",
            "screenControl": "off", "validUntil": null, "account": "own"
        }}),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        selected.folders,
        FolderChoice::Selected {
            paths: vec!["/home/u/Docs".into()],
            connections: false
        }
    );
    assert!(selected.write);
    assert_eq!(selected.commands, CapabilityDecision::Ask);
    assert_eq!(selected.screen_view, CapabilityDecision::Allow);
    assert_eq!(selected.valid_until, None);

    // Neither preset nor rights: the call keeps its default.
    assert!(rights_arg(&json!({}), None).unwrap().is_none());
    assert!(rights_arg(&json!({"preset": null, "rights": null}), None)
        .unwrap()
        .is_none());
    // `none`/`all` without paths, as `rightsFieldsJson` sends them.
    let none = rights_arg(&json!({"rights": {"folders": {"kind": "none"}}}), None)
        .unwrap()
        .unwrap();
    assert_eq!(none, ConnectionRights::none());
}
