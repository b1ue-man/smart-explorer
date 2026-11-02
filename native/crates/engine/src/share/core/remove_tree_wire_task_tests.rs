use super::FsRemoveTreeMessage;
use crate::share::wire::{FsHostFeatures, FsRequest, FsResponse};

#[test]
fn delete_speed_task_remove_tree_request_round_trips_as_a_write() {
    let request = FsRequest::RemoveTree {
        path: "/Docs/old".into(),
    };
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(encoded, r#"{"op":"remove_tree","path":"/Docs/old"}"#);
    let FsRequest::RemoveTree { path } = serde_json::from_str::<FsRequest>(&encoded).unwrap()
    else {
        panic!("remove tree expected");
    };
    assert_eq!(path, "/Docs/old");
    // Refused on read-only exports and for peers without write right.
    assert!(request.mutates_filesystem());
    assert!(!request.is_batch());
    assert!(!request.is_transfer_v1());
}

#[test]
fn delete_speed_task_remove_tree_messages_round_trip() {
    let messages = [
        FsRemoveTreeMessage::Progress {
            applying: false,
            planned: 3,
            removed: 0,
        },
        FsRemoveTreeMessage::Progress {
            applying: true,
            planned: 3,
            removed: 2,
        },
        FsRemoveTreeMessage::Done {
            planned: 3,
            removed: 3,
        },
    ];
    for message in messages {
        let response = FsResponse::RemoveTree { message };
        let encoded = serde_json::to_string(&response).unwrap();
        let FsResponse::RemoveTree { message: decoded } =
            serde_json::from_str::<FsResponse>(&encoded).unwrap()
        else {
            panic!("remove tree response expected: {encoded}");
        };
        assert_eq!(decoded, message);
    }
    let progress = serde_json::to_string(&FsResponse::RemoveTree {
        message: FsRemoveTreeMessage::Progress {
            applying: true,
            planned: 9,
            removed: 4,
        },
    })
    .unwrap();
    assert_eq!(
        progress,
        r#"{"r":"remove_tree","message":{"m":"progress","applying":true,"planned":9,"removed":4}}"#
    );
    let done: FsResponse = serde_json::from_str(
        r#"{"r":"remove_tree","message":{"m":"done","planned":5,"removed":5}}"#,
    )
    .unwrap();
    assert!(matches!(
        done,
        FsResponse::RemoveTree {
            message: FsRemoveTreeMessage::Done {
                planned: 5,
                removed: 5
            }
        }
    ));
}

#[test]
fn delete_speed_task_host_offers_remove_tree_and_old_hosts_do_not() {
    assert!(FsHostFeatures::host().remove_tree_v1);
    assert!(FsHostFeatures::host().names().contains(&"remove_tree_v1"));
    let old_host: FsHostFeatures = serde_json::from_str(r#"{"watch_v1":true}"#).unwrap();
    assert!(!old_host.remove_tree_v1);
    let encoded = serde_json::to_string(&FsHostFeatures::host()).unwrap();
    assert!(encoded.contains(r#""remove_tree_v1":true"#), "{encoded}");
    let without = FsHostFeatures {
        remove_tree_v1: false,
        ..FsHostFeatures::host()
    };
    assert!(!serde_json::to_string(&without)
        .unwrap()
        .contains("remove_tree_v1"));
}
