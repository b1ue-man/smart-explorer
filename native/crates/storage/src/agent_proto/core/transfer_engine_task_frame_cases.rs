//! The frame round trips that are cases of
//! `transfer_engine_task_new_frames_roundtrip_with_exact_lengths`: the
//! remove-tree frame, the LZ4 frames, the keyed walk, the times of a batch
//! put, the attribute listing and the walk with attributes.
use super::super::range_lz4_tests::{random, text};
use super::super::{
    pack_lz4, pack_raw, query, Frame, WireHashEntry, WireHashMeta, WireOmission, MAX_RAW,
};
use std::io::ErrorKind;

/// Round trip with request id 7 and the exact wire length, as the walk-meta frames use it.
fn roundtrip_id7(frame: Frame) -> Frame {
    let bytes = frame.encode(7).unwrap();
    assert_eq!(bytes.len(), frame.wire_len().unwrap());
    let (id, decoded) = Frame::decode(&bytes).unwrap();
    assert_eq!(id, 7);
    decoded
}

/// Round trip with request id 11 and the exact wire length, as the keyed walk frames use it.
fn roundtrip_id11(frame: Frame) -> Frame {
    let bytes = frame.encode(11).unwrap();
    assert_eq!(bytes.len(), frame.wire_len().unwrap());
    let (id, decoded) = Frame::decode(&bytes).unwrap();
    assert_eq!(id, 11);
    decoded
}

pub(super) fn delete_speed_task_remove_tree_frame_round_trips_with_its_query() {
    assert_eq!(query::REMOVE_TREE, 6);
    let frame = Frame::RemoveTree("/srv/daten/Ordner mit ü".into());
    let encoded = frame.encode(42).unwrap();
    assert_eq!(encoded[8], 62, "tag after the request id");
    assert_eq!(encoded.len(), frame.wire_len().unwrap());
    assert_eq!(Frame::decode(&encoded).unwrap(), (42, frame));

    let question = Frame::Query {
        kind: query::REMOVE_TREE,
        path: "/srv".into(),
    };
    let encoded = question.encode(7).unwrap();
    assert_eq!(Frame::decode(&encoded).unwrap(), (7, question));

    let mut longer = Frame::RemoveTree("/a".into()).encode(1).unwrap();
    longer.push(0);
    assert!(Frame::decode(&longer).is_err());
    let full = Frame::RemoveTree("/abc".into()).encode(1).unwrap();
    assert!(Frame::decode(&full[..full.len() - 1]).is_err());
}

pub(super) fn speed_task_agent_lz4_frames_roundtrip_on_the_wire() {
    let block = pack_lz4(&text(MAX_RAW)).expect("packs");
    for frame in [
        Frame::DataLz4(block),
        Frame::DataLz4(pack_raw(&random(1000))),
        Frame::ReadLz4 {
            path: "/data/x".into(),
            offset: 5,
            len: 77,
        },
    ] {
        let bytes = frame.encode(9).expect("encode");
        assert_eq!(Frame::decode(&bytes).expect("decode"), (9, frame));
    }
    let oversized = Frame::DataLz4(vec![0; MAX_RAW + 6]);
    assert_eq!(
        oversized.encode(1).expect_err("too long").kind(),
        ErrorKind::InvalidData
    );
}

pub(super) fn abgleich_task_walk_key_frames_round_trip() {
    let mut known = vec![3u8; 32];
    known[16] = 9;
    for frame in [
        Frame::WalkKeyed {
            root: "/srv/data".into(),
            algorithm: 1,
            min_bytes: 0,
            known: known.clone(),
            keys_only: false,
            scope: Default::default(),
        },
        Frame::WalkKeyed {
            root: "/".into(),
            algorithm: 0,
            min_bytes: 5,
            known: Vec::new(),
            keys_only: true,
            scope: Default::default(),
        },
        Frame::ListRuns {
            root: "/srv".into(),
            algorithm: 1,
            folders: vec![String::new(), "a/b".into(), "ä".into()],
            scope: Default::default(),
        },
        Frame::FolderKey {
            rel: "a/b".into(),
            key: [0xab; 32],
            same: true,
            tally: None,
        },
        Frame::FolderKey {
            rel: String::new(),
            key: [1; 32],
            same: false,
            tally: Some(crate::agent_proto::KeyTally {
                files: 300,
                bytes: u64::MAX,
                omitted: 0,
                untimed: 2,
                height: 17,
            }),
        },
        Frame::FolderRun("a/ä b".into()),
        Frame::ListDirMeta("/home/u".into()),
        Frame::StatMeta("/home/u/.x".into()),
    ] {
        assert_eq!(roundtrip_id11(frame.clone()), frame);
    }
}

pub(super) fn abgleich_task_batch_put_times_travel_as_tag_81() {
    use crate::agent_proto::BatchEntry;
    let entry = |mtime_ms| BatchEntry {
        path: "/srv/a.txt".into(),
        size: 3,
        nonce: 9,
        mtime_ms,
    };
    let plain = Frame::BatchPut {
        entries: vec![entry(None)],
    };
    let bytes = plain.encode(1).unwrap();
    assert_eq!(bytes[8], 35, "a batch without times keeps its former tag");
    assert_eq!(roundtrip_id7(plain.clone()), plain);
    let timed = Frame::BatchPut {
        entries: vec![entry(Some(1_600_000_000_123)), entry(None)],
    };
    let bytes = timed.encode(1).unwrap();
    assert_eq!(bytes[8], 81);
    assert_eq!(roundtrip_id7(timed.clone()), timed);
}

pub(super) fn abgleich_task_list_meta_travels_as_tag_82_behind_its_label() {
    let request = Frame::ListMeta("/srv/daten".into());
    assert_eq!(roundtrip_id7(request.clone()), request);
    let bytes = request.encode(9).unwrap();
    assert_eq!(bytes[8], 82, "tag after the request id");
    let service = crate::agent_proto::ServerFeatures::parse(
        "0.5.190+ext-v1.walk-meta-v1.list-meta-v1 worker",
    );
    assert!(service.list_meta && service.service);
    let older = crate::agent_proto::ServerFeatures::parse("0.5.189+ext-v1 worker");
    assert!(!older.list_meta);
}

pub(super) fn abgleich_task_walk_meta_frames_round_trip() {
    for meta in [false, true] {
        let request = Frame::WalkHashed3 {
            root: "/srv/data".into(),
            algorithm: 1,
            min_bytes: 9,
            meta,
            scope: Default::default(),
        };
        assert_eq!(roundtrip_id7(request.clone()), request);
    }
    let part = Frame::HashPart {
        entries: vec![
            WireHashEntry {
                rel: "a".into(),
                is_dir: true,
                size: 4096,
                mtime_ms: -5,
                digest: None,
                meta: Some(WireHashMeta {
                    hidden: true,
                    system: true,
                    btime_ms: 1_700_000_000_000,
                    is_symlink: false,
                    special: false,
                }),
            },
            WireHashEntry {
                rel: "a/link".into(),
                meta: Some(WireHashMeta {
                    is_symlink: true,
                    ..WireHashMeta::default()
                }),
                ..WireHashEntry::default()
            },
            WireHashEntry {
                rel: "a/f.txt".into(),
                size: 3,
                mtime_ms: 1,
                digest: Some("900150983cd24fb0d6963f7d28e17f72".into()),
                ..WireHashEntry::default()
            },
        ],
        omitted: vec![WireOmission {
            rel: "a/x".into(),
            reason: 2,
            detail: "denied".into(),
        }],
    };
    assert_eq!(roundtrip_id7(part.clone()), part);
    assert!(crate::agent_proto::charged(&part));
}
