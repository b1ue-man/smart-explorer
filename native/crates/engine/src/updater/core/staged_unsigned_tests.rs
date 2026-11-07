//! The staging manifest records why a bundle has no release signature (E12;
//! suite expectation E12-serial of the review 2026-11-07): the field is
//! absent for a signed bundle, so manifests written before signing existed
//! and manifests of signed releases keep one form.
use std::path::PathBuf;

use serde_json::json;

use super::{StagedUpdate, VerifiedPayload};
use crate::updater::feed_trust::UnsignedFeed;

fn payload(name: &str, digit: char) -> VerifiedPayload {
    VerifiedPayload::new(
        PathBuf::from(format!("/staged/{name}")),
        digit.to_string().repeat(64),
    )
    .expect("a payload with a SHA-256")
}

fn bundle(with_launcher: bool) -> StagedUpdate {
    StagedUpdate::new(
        "1.2.3".into(),
        payload("app", 'a'),
        payload("helper", 'b'),
        payload("cli", 'c'),
        with_launcher.then(|| payload("launcher", 'd')),
    )
    .expect("a staged update")
}

fn field_names(value: &serde_json::Value) -> Vec<&str> {
    let mut names: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn lokal_task_staged_update_without_a_signature_note_keeps_its_old_form() {
    let signed = bundle(true);
    assert_eq!(signed.unsigned(), None);
    let written = serde_json::to_value(&signed).expect("serialized");
    assert_eq!(
        field_names(&written),
        ["app", "cli", "helper", "launcher", "schema", "version"]
    );
    let without_launcher = serde_json::to_value(bundle(false)).expect("serialized");
    assert_eq!(
        field_names(&without_launcher),
        ["app", "cli", "helper", "schema", "version"]
    );

    // A manifest written before signing existed is read, with no note.
    let before = json!({
        "schema": 2,
        "version": "1.2.3",
        "app": {"path": "/staged/app", "sha256": "a".repeat(64)},
        "helper": {"path": "/staged/helper", "sha256": "b".repeat(64)},
        "cli": {"path": "/staged/cli", "sha256": "c".repeat(64)},
        "launcher": {"path": "/staged/launcher", "sha256": "d".repeat(64)}
    });
    let read: StagedUpdate = serde_json::from_value(before).expect("the old form is read");
    assert_eq!(read, signed);
    assert_eq!(read.unsigned(), None);

    // `null` is no note either; an unknown note is refused, not ignored.
    let mut with_null = written.clone();
    with_null["unsigned"] = serde_json::Value::Null;
    let read: StagedUpdate = serde_json::from_value(with_null).expect("a null note");
    assert_eq!(read.unsigned(), None);
    let mut unknown = written;
    unknown["unsigned"] = json!("something_else");
    assert!(serde_json::from_value::<StagedUpdate>(unknown).is_err());
}

#[test]
fn lokal_task_staged_update_records_why_it_is_unsigned_and_reads_it_back() {
    for (kind, text) in [
        (UnsignedFeed::PlainHttp, "plain_http"),
        (UnsignedFeed::OfficialRelease, "official_release"),
        (UnsignedFeed::Custom, "custom"),
    ] {
        let noted = bundle(true).with_unsigned(Some(kind));
        assert_eq!(noted.unsigned(), Some(kind), "{text}");
        let written = serde_json::to_value(&noted).expect("serialized");
        assert_eq!(written["unsigned"], text);
        let read: StagedUpdate = serde_json::from_value(written).expect("read back");
        assert_eq!(read, noted, "{text}");
        assert_eq!(read.unsigned(), Some(kind), "{text}");
        // The note is part of the bundle's identity: the staging manifest of
        // a signed bundle is not equal to the unsigned one.
        assert_ne!(read, bundle(true), "{text}");
        assert_eq!(noted.with_unsigned(None), bundle(true), "{text}");
    }
}
