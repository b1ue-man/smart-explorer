use super::negotiated;
use std::{cell::Cell, io};
use crate::share::{
    export_config::ExportAccess,
    fs::{ShareExportConfig, SharedRoot},
    fs_access::{reversible_replace::validate, FsAccess},
    fs_request::FsReversibleReplace,
    wire::{FsHostFeatures, FsRequest, FsResponse},
};

fn request() -> FsReversibleReplace {
    FsReversibleReplace {
        staged: "/Docs/file.se-transfer-0123456789abcdef".into(),
        destination: "/Docs/file".into(),
        retained: "/Docs/.se-replace-fedcba9876543210".into(),
    }
}

#[test]
fn review_task_h_replace_no_feature_is_mutation_free() {
    let called = Cell::new(false);
    let released = Cell::new(false);
    let result = negotiated(
        &request(),
        false,
        true,
        |_| {
            called.set(true);
            Ok(FsResponse::ReversibleReplaced { replaced: true })
        },
        || released.set(true),
    )
    .unwrap();
    assert!(!result);
    assert!(!called.get());
    assert!(!released.get());
    let legacy: FsHostFeatures = serde_json::from_str("{}").unwrap();
    assert!(!legacy.reversible_replace_v1);
    assert!(FsHostFeatures::host().reversible_replace_v1);
}

#[test]
fn review_task_h_replace_idle_close_and_lost_ack_never_replay_or_release() {
    for kind in [io::ErrorKind::ConnectionAborted, io::ErrorKind::UnexpectedEof] {
        let fixture = tempfile::tempdir().unwrap();
        let stage = fixture.path().join("stage");
        let destination = fixture.path().join("destination");
        let retained = fixture.path().join("retained");
        std::fs::write(&stage, b"prepared").unwrap();
        std::fs::write(&destination, b"original").unwrap();
        let calls = Cell::new(0);
        let released = Cell::new(false);
        let result = negotiated(
            &request(),
            true,
            true,
            |_| {
                calls.set(calls.get() + 1);
                // Publication may have committed before the response vanished.
                std::fs::rename(&destination, &retained)?;
                std::fs::rename(&stage, &destination)?;
                Err(io::Error::new(kind, "result unavailable after operation"))
            },
            || released.set(true),
        );
        assert_eq!(result.unwrap_err().kind(), kind);
        assert_eq!(calls.get(), 1);
        assert_eq!(std::fs::read(&retained).unwrap(), b"original");
        assert_eq!(std::fs::read(&destination).unwrap(), b"prepared");
        assert!(!released.get());
    }
}

#[test]
fn review_task_h_replace_only_confirmed_true_releases_own_stage() {
    for replaced in [false, true] {
        let released = Cell::new(false);
        let result = negotiated(
            &request(),
            true,
            true,
            |_| Ok(FsResponse::ReversibleReplaced { replaced }),
            || released.set(true),
        )
        .unwrap();
        assert_eq!(result, replaced);
        assert_eq!(released.get(), replaced);
    }
    let result = negotiated(
        &request(),
        true,
        false,
        |_| panic!("foreign stage must not reach transport"),
        || panic!("foreign stage must not be released"),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    let released = Cell::new(false);
    let result = negotiated(
        &request(),
        true,
        true,
        |_| Ok(FsResponse::Ok),
        || released.set(true),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(!released.get());
}

#[test]
fn review_task_h_replace_readonly_host_preserves_all_objects() {
    let fixture = tempfile::tempdir().unwrap();
    let stage_name = "file.se-transfer-0123456789abcdef";
    std::fs::write(fixture.path().join(stage_name), b"prepared").unwrap();
    std::fs::write(fixture.path().join("file"), b"original").unwrap();
    let access = FsAccess::dynamic(ShareExportConfig {
        roots: vec![SharedRoot::new(
            "Docs",
            fixture.path().to_string_lossy().replace('\\', "/"),
        )
        .with_access(ExportAccess::ReadOnly)],
        ..Default::default()
    });
    let error = access.replace_staged_reversible(&request()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ReadOnlyFilesystem);
    assert_eq!(std::fs::read(fixture.path().join(stage_name)).unwrap(), b"prepared");
    assert_eq!(std::fs::read(fixture.path().join("file")).unwrap(), b"original");
    assert!(!fixture.path().join(".se-replace-fedcba9876543210").exists());
}

#[test]
fn review_task_h_replace_retained_contract_keeps_literal_parent_and_nonce() {
    validate(&request()).unwrap();
    let mut value = request();
    value.staged = "/Verbindungen/Drive/stored%2520/file.se-transfer-0123456789abcdef".into();
    value.destination = "/Verbindungen/Drive/stored%2520/%2561ux.c".into();
    value.retained = "/Verbindungen/Drive/stored%2520/.se-replace-fedcba9876543210".into();
    validate(&value).unwrap();
    for retained in [
        "/Docs/file.se-replace-fedcba9876543210",
        "/Docs/.se-replace-fedcba987654321",
        "/Docs/.se-replace-FEDCBA9876543210",
        "/Other/.se-replace-fedcba9876543210",
    ] {
        let mut invalid = request();
        invalid.retained = retained.into();
        assert!(validate(&invalid).is_err());
    }
    let mut invalid = request();
    invalid.staged = invalid.destination.clone();
    assert!(validate(&invalid).is_err());
    invalid = request();
    invalid.staged = "/Other/file.se-transfer-0123456789abcdef".into();
    assert!(validate(&invalid).is_err());
}

#[test]
fn review_task_h_replace_wire_classifies_mutation_and_requires_explicit_boolean() {
    let wire_request = FsRequest::ReplaceStagedReversible(request());
    assert!(wire_request.mutates_filesystem());
    assert!(!wire_request.is_transfer_v1());
    let encoded = serde_json::to_value(&wire_request).unwrap();
    assert_eq!(encoded["op"], "replace_staged_reversible");
    assert_eq!(encoded["retained"], request().retained);
    let encoded = serde_json::to_value(FsResponse::ReversibleReplaced { replaced: true }).unwrap();
    assert_eq!(encoded["r"], "reversible_replaced");
    assert_eq!(encoded["replaced"], true);
    assert!(serde_json::from_str::<FsResponse>(r#"{"r":"reversible_replaced"}"#).is_err());
}
