use super::*;

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum LegacyRequest {
    Capabilities {
        path: String,
        #[serde(default)]
        acquire_lease: bool,
    },
}

#[test]
fn remote_drive_task_mount_request_id_is_additive_for_legacy_peer() {
    let request = FsRequest::Capabilities {
        path: "/Docs".into(),
        acquire_lease: true,
        lease_request_id: Some("request-a".into()),
    };
    let json = serde_json::to_string(&request).unwrap();
    let LegacyRequest::Capabilities {
        path,
        acquire_lease,
    } = serde_json::from_str(&json).unwrap();
    assert_eq!(path, "/Docs");
    assert!(acquire_lease);

    let decoded: FsRequest =
        serde_json::from_str(r#"{"op":"capabilities","path":"/Docs","acquire_lease":true}"#)
            .unwrap();
    assert!(matches!(
        decoded,
        FsRequest::Capabilities {
            lease_request_id: None,
            ..
        }
    ));
}

#[test]
fn remote_drive_task_legacy_peer_rejects_unknown_release_without_reinterpreting_it() {
    let json = serde_json::to_string(&FsRequest::ReleaseLease).unwrap();
    assert!(serde_json::from_str::<LegacyRequest>(&json).is_err());
}
