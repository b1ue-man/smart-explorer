use super::*;
use crate::agent_proto::{Frame, WireNode};

#[test]
fn share_remote_task_storage_snapshot_finished_tree_and_legacy_fallback() {
    let tree = fixture_tree();
    let totals = validate_tree(&tree).unwrap();
    assert_eq!(totals, WalkTotals {
        files: 1,
        dirs: 2,
        bytes: 7,
    });
    assert_eq!(reported_totals(1, 2, 7, 3).unwrap(), totals);
    require_monotonic(WalkTotals::default(), totals).unwrap();

    let encoded = Frame::Tree(tree.clone()).encode(0).unwrap();
    assert!(!encoded.is_empty());
    assert!(encoded.len() <= MAX_SNAPSHOT_BYTES);
    let digest = sha256(&encoded);
    assert_ne!(digest, [0; 32]);
    let (request_id, frame) = Frame::decode(&encoded).unwrap();
    assert_eq!(request_id, 0);
    assert_eq!(frame, Frame::Tree(tree));

    let legacy: super::super::wire::FsResponse = serde_json::from_str(
        r#"{"r":"capabilities","capabilities":{"create":false,"replace":false,"namespace_replace":false}}"#,
    )
    .unwrap();
    let super::super::wire::FsResponse::Capabilities {
        storage_snapshot_v1,
        ..
    } = legacy
    else {
        panic!("legacy capability response changed kind");
    };
    assert!(!storage_snapshot_v1);

    let advertised = super::super::wire::FsResponse::Capabilities {
        capabilities: Default::default(),
        contract_version: 3,
        root_confined: true,
        lease: None,
        storage_snapshot_v1: true,
    };
    assert!(matches!(
        advertised,
        super::super::wire::FsResponse::Capabilities {
            storage_snapshot_v1: true,
            ..
        }
    ));
}

#[test]
fn share_remote_task_storage_snapshot_corruption_is_rejected() {
    let tree = fixture_tree();
    let encoded = Frame::Tree(tree.clone()).encode(0).unwrap();
    let expected_digest = sha256(&encoded);
    let mut corrupted = encoded.clone();
    *corrupted.last_mut().unwrap() ^= 1;
    assert_ne!(sha256(&corrupted), expected_digest);

    assert!(reported_totals(1, 1, 7, 3).is_err());
    assert!(require_monotonic(
        WalkTotals {
            files: 2,
            dirs: 2,
            bytes: 8,
        },
        WalkTotals {
            files: 1,
            dirs: 2,
            bytes: 7,
        },
    )
    .is_err());

    let mut wrong_size = tree.clone();
    wrong_size.size += 1;
    assert!(validate_tree(&wrong_size).is_err());
    let file_with_child = WireNode {
        name: "/".into(),
        size: 7,
        is_dir: true,
        children: vec![WireNode {
            name: "bad".into(),
            size: 7,
            is_dir: false,
            children: vec![WireNode {
                name: "child".into(),
                size: 1,
                is_dir: false,
                children: Vec::new(),
            }],
        }],
    };
    assert!(validate_tree(&file_with_child).is_err());
}

fn fixture_tree() -> WireNode {
    WireNode {
        name: "/".into(),
        size: 7,
        is_dir: true,
        children: vec![
            WireNode {
                name: "folder".into(),
                size: 0,
                is_dir: true,
                children: Vec::new(),
            },
            WireNode {
                name: "file.txt".into(),
                size: 7,
                is_dir: false,
                children: Vec::new(),
            },
        ],
    }
}
