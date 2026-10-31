use crate::agent_proto::{Frame, WireHashEntry, WireHashMeta, WireOmission};

fn roundtrip(frame: Frame) -> Frame {
    let bytes = frame.encode(7).unwrap();
    assert_eq!(bytes.len(), frame.wire_len().unwrap());
    let (id, decoded) = Frame::decode(&bytes).unwrap();
    assert_eq!(id, 7);
    decoded
}

#[test]
fn abgleich_task_walk_meta_frames_round_trip() {
    for meta in [false, true] {
        let request = Frame::WalkHashed3 {
            root: "/srv/data".into(),
            algorithm: 1,
            min_bytes: 9,
            meta,
        };
        assert_eq!(roundtrip(request.clone()), request);
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
    assert_eq!(roundtrip(part.clone()), part);
    assert!(crate::agent_proto::charged(&part));
}

#[test]
fn abgleich_task_walk_meta_frames_refuse_unknown_bits() {
    let part = Frame::HashPart {
        entries: vec![WireHashEntry {
            rel: "f".into(),
            ..WireHashEntry::default()
        }],
        omitted: Vec::new(),
    };
    let mut bytes = part.encode(1).unwrap();
    // Request id (8), tag (1), count (4), rel length (4) and "f": the flags.
    let flags = 8 + 1 + 4 + 4 + 1;
    bytes[flags] = 4; // `hidden` without the attribute flag
    assert!(Frame::decode(&bytes).is_err());
    let mut request = Frame::WalkHashed3 {
        root: "/".into(),
        algorithm: 0,
        min_bytes: 0,
        meta: true,
    }
    .encode(1)
    .unwrap();
    *request.last_mut().unwrap() = 2;
    assert!(Frame::decode(&request).is_err());
}

#[test]
fn abgleich_task_batch_put_times_travel_as_tag_81() {
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
    assert_eq!(roundtrip(plain.clone()), plain);
    let timed = Frame::BatchPut {
        entries: vec![entry(Some(1_600_000_000_123)), entry(None)],
    };
    let bytes = timed.encode(1).unwrap();
    assert_eq!(bytes[8], 81);
    assert_eq!(roundtrip(timed.clone()), timed);
}
