use super::fs_request::{FsHashAlgo, FsReversibleReplace, FsStageDurability};
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

fn one_of_each() -> Vec<FsRequest> {
    let path = || "/A/x".to_string();
    vec![
        FsRequest::Capabilities {
            path: path(),
            acquire_lease: true,
            lease_request_id: None,
        },
        FsRequest::ReleaseLease,
        FsRequest::ListDir { path: path() },
        FsRequest::Stat { path: path() },
        FsRequest::SyncChildPath {
            parent: path(),
            literal_name: "100%.pdf".into(),
        },
        FsRequest::WalkTree { path: path() },
        FsRequest::StorageSnapshot { path: path() },
        FsRequest::StorageAnalysis(FsStorageAnalysis {
            path: path(),
            ..Default::default()
        }),
        FsRequest::Read { path: path() },
        FsRequest::Write { path: path() },
        FsRequest::WriteNew { path: path() },
        FsRequest::WriteDone,
        FsRequest::MkdirAll { path: path() },
        FsRequest::Rename {
            src: path(),
            dst: path(),
        },
        FsRequest::RenameNoReplace {
            src: path(),
            dst: path(),
        },
        FsRequest::PromoteStaged {
            staged: path(),
            destination: path(),
        },
        FsRequest::ReplaceStagedReversible(FsReversibleReplace {
            staged: "/A/x.se-bisync-0123456789abcdef".into(),
            destination: path(),
            retained: "/A/.se-replace-0123456789abcdef".into(),
        }),
        FsRequest::CopyFile {
            src: path(),
            dst: path(),
        },
        FsRequest::RemoveFile { path: path() },
        FsRequest::RemoveDir { path: path() },
        FsRequest::PutBatch {
            nonce: "0123456789abcdef".into(),
            entries: Vec::new(),
        },
        FsRequest::PutBatchStatus {
            nonce: "0123456789abcdef".into(),
        },
        FsRequest::GetBatch { items: Vec::new() },
        FsRequest::ReadAt {
            path: path(),
            id: None,
            offset: 0,
        },
        FsRequest::CreateDir {
            path: path(),
            exclusive: false,
        },
        FsRequest::PromoteNoReplace {
            staged: path(),
            destination: path(),
            copy: false,
        },
        FsRequest::DiscardStage { path: path() },
        FsRequest::DuplicateSearch(FsDuplicateSearch {
            path: path(),
            min_bytes: 1,
            request_id: None,
        }),
        FsRequest::HashWalk(FsHashWalk {
            path: path(),
            algo: Some(FsHashAlgo::Md5),
            min_bytes: 0,
        }),
        FsRequest::ListDirBatch(FsListBatch {
            path: path(),
            cursor: None,
        }),
        FsRequest::Recycle(FsRecycle {
            path: path(),
            expected_size: 1,
            expected_sha256: None,
        }),
        FsRequest::FinishStage(FsStageFinish {
            staged: path(),
            mtime_ms: Some(1),
            mode: None,
            durability: FsStageDurability::Now,
        }),
        FsRequest::SyncFilesystem(FsSyncFilesystem { path: path() }),
        FsRequest::WatchExport(FsWatch { path: path() }),
    ]
}

fn op(request: &FsRequest) -> String {
    serde_json::to_value(request).unwrap()["op"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn review_task_every_request_is_classified_as_read_or_write() {
    let requests = one_of_each();
    let ops: std::collections::BTreeSet<String> = requests.iter().map(op).collect();
    assert_eq!(ops.len(), requests.len(), "one request of every kind");
    let writes: Vec<String> = requests
        .iter()
        .filter(|request| request.mutates_filesystem())
        .map(op)
        .collect();
    assert_eq!(
        writes,
        [
            "write",
            "write_new",
            "write_done",
            "mkdir_all",
            "rename",
            "rename_no_replace",
            "promote_staged",
            "replace_staged_reversible",
            "copy_file",
            "remove_file",
            "remove_dir",
            "put_batch",
            "create_dir",
            "promote_no_replace",
            "discard_stage",
            "recycle",
            "finish_stage",
            "sync_filesystem",
        ]
    );
}

#[test]
fn review_task_rv1_request_fields_stay_additive_on_the_wire() {
    let plain = FsRequest::StorageAnalysis(FsStorageAnalysis {
        path: "/A".into(),
        ..Default::default()
    });
    assert_eq!(
        serde_json::to_string(&plain).unwrap(),
        r#"{"op":"storage_analysis","path":"/A"}"#
    );
    let legacy: FsRequest =
        serde_json::from_str(r#"{"op":"storage_analysis","path":"/A"}"#).unwrap();
    let FsRequest::StorageAnalysis(request) = legacy else {
        panic!("storage analysis expected");
    };
    assert_eq!(request.path, "/A");
    assert_eq!(
        (request.request_id, request.node_budget, request.compress),
        (None, None, false)
    );

    let search = FsRequest::DuplicateSearch(FsDuplicateSearch {
        path: "/A".into(),
        min_bytes: 1 << 20,
        request_id: None,
    });
    assert_eq!(
        serde_json::to_string(&search).unwrap(),
        r#"{"op":"duplicate_search","path":"/A","min_bytes":1048576}"#
    );
    let recycle = FsRequest::Recycle(FsRecycle {
        path: "/A/x".into(),
        expected_size: 7,
        expected_sha256: Some("ab".repeat(32)),
    });
    let encoded = serde_json::to_string(&recycle).unwrap();
    let FsRequest::Recycle(decoded) = serde_json::from_str::<FsRequest>(&encoded).unwrap() else {
        panic!("recycle expected");
    };
    assert_eq!(decoded.expected_sha256, Some("ab".repeat(32)));
    let future: FsRequest =
        serde_json::from_str(r#"{"op":"hash_walk","path":"/A","algo":"blake3"}"#).unwrap();
    let FsRequest::HashWalk(walk) = future else {
        panic!("hash walk expected");
    };
    assert_eq!((walk.algo, walk.min_bytes), (Some(FsHashAlgo::Unknown), 0));
    let finish: FsRequest =
        serde_json::from_str(r#"{"op":"finish_stage","staged":"/A/x.se-bisync-0123456789abcdef"}"#)
            .unwrap();
    let FsRequest::FinishStage(finish) = finish else {
        panic!("finish stage expected");
    };
    assert_eq!(
        (finish.mtime_ms, finish.mode, finish.durability),
        (None, None, FsStageDurability::NotRequired)
    );
}

#[test]
fn review_task_older_host_rejects_rv1_requests_instead_of_misreading_them() {
    #[derive(serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    #[allow(dead_code)]
    enum OlderRequest {
        ListDir { path: String },
        StorageAnalysis { path: String },
    }
    for request in one_of_each().iter().filter(|request| {
        matches!(
            request,
            FsRequest::DuplicateSearch(_)
                | FsRequest::HashWalk(_)
                | FsRequest::ListDirBatch(_)
                | FsRequest::Recycle(_)
                | FsRequest::FinishStage(_)
                | FsRequest::SyncFilesystem(_)
                | FsRequest::WatchExport(_)
                | FsRequest::SyncChildPath { .. }
                | FsRequest::ReplaceStagedReversible(_)
        )
    }) {
        let json = serde_json::to_string(request).unwrap();
        assert!(
            serde_json::from_str::<OlderRequest>(&json).is_err(),
            "{json}"
        );
    }
    let analysis = serde_json::to_string(&FsRequest::StorageAnalysis(FsStorageAnalysis {
        path: "/A".into(),
        request_id: Some("0123456789abcdef".into()),
        node_budget: Some(1000),
        compress: true,
    }))
    .unwrap();
    assert!(serde_json::from_str::<OlderRequest>(&analysis).is_ok());
}

#[test]
fn review_task_special_entries_and_host_features_stay_additive() {
    let legacy: FsMeta = serde_json::from_str(
        r#"{"name":"pipe","is_dir":false,"is_symlink":false,"size":0,"mtime_ms":0,
            "btime_ms":0,"hidden":false,"system":false,"id":null}"#,
    )
    .unwrap();
    assert!(!legacy.special);
    let mut special = legacy.clone();
    special.special = true;
    let encoded = serde_json::to_string(&special).unwrap();
    assert!(encoded.contains(r#""special":true"#), "{encoded}");
    assert!(!serde_json::to_string(&legacy).unwrap().contains("special"));
    assert!(serde_json::from_str::<FsMeta>(&encoded).unwrap().special);

    let old_host: FsWriteCapabilities =
        serde_json::from_str(r#"{"create":true,"replace":true,"namespace_replace":true}"#).unwrap();
    assert!(old_host.features.is_absent());
    assert_eq!(old_host.access, None);
    let mut current = FsWriteCapabilities::from(crate::vfs::StagedWriteCapabilities::default());
    current.features.watch_v1 = true;
    current.features.export_access_v1 = true;
    current.access = Some(crate::share::ExportAccess::ReadOnly);
    let encoded = serde_json::to_string(&current).unwrap();
    assert!(
        encoded.contains(r#""features":{"watch_v1":true,"export_access_v1":true}"#),
        "{encoded}"
    );
    assert!(encoded.contains(r#""access":"read_only""#), "{encoded}");
    let decoded: FsWriteCapabilities = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, current);
    assert_eq!(decoded.features.names(), ["watch_v1", "export_access_v1"]);
}
