//! Transfer-engine protocol additions: new frames, credit accounting,
//! capability labels and batch bounds.
use super::credit::encoded_cost;
use super::range_lz4_tests::{random, text};
use super::{
    credit_cost, get_item_len, has_link_aware_hash, parse_busy, put_entry_len, server_version,
    split_batch, window_target, write_frame, BatchEntry, BatchItem, Frame, RecvWindow, SendCredit,
    ServerFeatures, StreamCount, BATCH_HEADER_MAX, BATCH_MAX_BYTES, BATCH_MAX_FILES, CHUNK,
    CREDIT_CONNECTION_BUDGET, CREDIT_INITIAL, CREDIT_REQUEST_LIMIT, CREDIT_WINDOW_MAX,
};
use super::{pack_lz4, pack_raw, query, WireHashEntry, WireHashMeta, WireOmission, MAX_RAW};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

fn new_frames() -> Vec<Frame> {
    vec![
        Frame::Credit { bytes: 1 << 40 },
        Frame::BatchPut {
            entries: vec![
                BatchEntry {
                    path: "/a/ä.txt".into(),
                    size: 0,
                    nonce: u64::MAX,
                    mtime_ms: None,
                },
                BatchEntry {
                    path: "/b".into(),
                    size: 7,
                    nonce: 1,
                    mtime_ms: None,
                },
            ],
        },
        Frame::BatchPut { entries: vec![] },
        Frame::BatchGet {
            items: vec![
                BatchItem {
                    path: "/x".into(),
                    id: Some("drive-id".into()),
                    size: 3,
                },
                BatchItem {
                    path: "/y".into(),
                    id: None,
                    size: 0,
                },
            ],
        },
        Frame::ItemBegin {
            index: 7,
            size: 1 << 33,
        },
        Frame::ItemEnd {
            index: 1,
            error: None,
        },
        Frame::ItemEnd {
            index: 2,
            error: Some("Quelle geändert".into()),
        },
        Frame::ItemPublished {
            index: 3,
            path: "/dir/name (2).txt".into(),
        },
        Frame::ItemFailed {
            index: 4,
            message: "denied".into(),
        },
        Frame::CopyToStage {
            src: "/a".into(),
            stage: "/a.se-copy-1".into(),
            size: 9,
        },
        Frame::Copied(Some(9)),
        Frame::Copied(None),
        Frame::CreateDir {
            path: "/new".into(),
            exclusive: true,
        },
        Frame::DiscardStage("/a.se-copy-1".into()),
    ]
}

#[test]
fn transfer_engine_task_new_frames_roundtrip_with_exact_lengths() {
    let cases: [(&str, fn()); 7] = [
        (
            "transfer_engine_task_new_frames_roundtrip_with_exact_lengths",
            || {
                for frame in new_frames() {
                    let body = frame.encode(42).unwrap();
                    assert_eq!(body.len(), frame.wire_len().unwrap(), "{frame:?}");
                    assert_eq!(Frame::decode(&body).unwrap(), (42, frame.clone()));
                    let mut transport = Vec::new();
                    write_frame(&mut transport, 42, &frame).unwrap();
                    assert_eq!(&transport[4..], &body[..]);
                }
                // Truncated batch headers are refused, never over-allocated.
                let mut hostile = Vec::new();
                hostile.extend_from_slice(&1u64.to_le_bytes());
                hostile.push(35);
                hostile.extend_from_slice(&u32::MAX.to_le_bytes());
                assert_eq!(
                    Frame::decode(&hostile).unwrap_err().kind(),
                    ErrorKind::InvalidData
                );
                let mut unknown = Vec::new();
                unknown.extend_from_slice(&1u64.to_le_bytes());
                unknown.push(45);
                assert_eq!(
                    Frame::decode(&unknown).unwrap_err().kind(),
                    ErrorKind::InvalidData
                );
            },
        ),
        (
            "delete_speed_task_remove_tree_frame_round_trips_with_its_query",
            || {
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
            },
        ),
        ("speed_task_agent_lz4_frames_roundtrip_on_the_wire", || {
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
        }),
        ("abgleich_task_walk_key_frames_round_trip", || {
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
        }),
        ("abgleich_task_batch_put_times_travel_as_tag_81", || {
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
        }),
        (
            "abgleich_task_list_meta_travels_as_tag_82_behind_its_label",
            || {
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
            },
        ),
        ("abgleich_task_walk_meta_frames_round_trip", || {
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
        }),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn transfer_engine_task_credit_cost_matches_encoded_frames() {
    let charged = [
        Frame::Data(vec![1; CHUNK]),
        Frame::TreeEntry {
            rel: "a/b".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
        },
        Frame::Match {
            rel: "m".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
        },
        Frame::HashEntry {
            rel: "h".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
            md5: None,
        },
    ];
    for frame in charged {
        let mut transport = Vec::new();
        write_frame(&mut transport, 5, &frame).unwrap();
        let cost = credit_cost(&frame);
        assert!(cost > 0 && cost <= CREDIT_INITIAL / 2, "{frame:?}");
        assert_eq!(cost, frame.wire_len().unwrap() as u64);
        assert_eq!(encoded_cost(&transport), cost);
    }
    let free = new_frames().into_iter().chain([
        Frame::Ok,
        Frame::End,
        Frame::Err("x".into()),
        Frame::Progress { done: 1, total: 2 },
        Frame::Dir(Vec::new()),
        Frame::Cancel,
    ]);
    for frame in free {
        let mut transport = Vec::new();
        write_frame(&mut transport, 5, &frame).unwrap();
        assert_eq!(credit_cost(&frame), 0, "{frame:?}");
        assert_eq!(encoded_cost(&transport), 0, "{frame:?}");
    }
    assert_eq!(encoded_cost(&[0; 12]), 0);
}

#[test]
fn transfer_engine_task_credit_numbers_follow_the_budget() {
    assert_eq!(CREDIT_REQUEST_LIMIT, 64);
    assert_eq!(
        CREDIT_REQUEST_LIMIT as u64 * CREDIT_INITIAL,
        CREDIT_CONNECTION_BUDGET
    );
    assert_eq!(CREDIT_WINDOW_MAX, 32 * CHUNK as u64);
    assert_eq!(window_target(0), CREDIT_WINDOW_MAX);
    assert_eq!(window_target(8), CREDIT_WINDOW_MAX);
    assert_eq!(window_target(16), 4 * 1024 * 1024);
    assert_eq!(window_target(1000), CREDIT_INITIAL);
}

#[test]
fn transfer_engine_task_send_credit_waits_for_grants_and_wakes_on_close() {
    let credit = Arc::new(SendCredit::new());
    credit.take(CREDIT_INITIAL, None).unwrap();
    let waiting = credit.clone();
    let taker = std::thread::spawn(move || waiting.take(10, Some(Duration::from_secs(5))));
    std::thread::sleep(Duration::from_millis(50));
    credit.grant(10);
    taker.join().unwrap().unwrap();

    let started = Instant::now();
    let error = credit.take(1, Some(Duration::from_millis(50))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(50));

    let closing = credit.clone();
    let taker = std::thread::spawn(move || closing.take(1, None));
    std::thread::sleep(Duration::from_millis(50));
    credit.close();
    assert_eq!(
        taker.join().unwrap().unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
}

#[test]
fn transfer_engine_task_receive_window_grants_and_detects_violations() {
    let streams = Arc::new(StreamCount::default());
    let window = RecvWindow::new(streams.clone());
    assert!(window.receive(CREDIT_INITIAL));
    assert!(!window.receive(1), "more than the implicit credit arrived");

    let window = RecvWindow::new(streams.clone());
    assert!(window.receive(CREDIT_INITIAL / 2));
    let grant = window
        .consume(CREDIT_INITIAL / 2)
        .expect("first consumption grows");
    assert_eq!(streams.current(), 1);
    assert_eq!(grant, CREDIT_WINDOW_MAX - CREDIT_INITIAL / 2);
    assert!(window.receive(CREDIT_WINDOW_MAX - CREDIT_INITIAL / 2 + CREDIT_INITIAL / 2));
    assert!(
        window.consume(1024).is_none(),
        "outstanding credit is still high"
    );
    drop(window);
    assert_eq!(streams.current(), 0);
}

#[test]
fn transfer_engine_task_labels_announce_capabilities_and_keep_old_meanings() {
    let agent = ServerFeatures::parse(&server_version(true));
    assert!(agent.credit && agent.batch && agent.stage && agent.link_aware_hash);
    assert!(agent.batches());
    assert_eq!(agent.slots, Some(CREDIT_REQUEST_LIMIT));
    assert_eq!(agent.transfer_slots(), CREDIT_REQUEST_LIMIT - 2);
    assert!(has_link_aware_hash(&server_version(true)));

    let service = ServerFeatures::parse(&format!("{} worker", server_version(false)));
    assert!(service.service && service.credit && !service.batch && !service.batches());
    assert_eq!(service.transfer_slots(), CREDIT_REQUEST_LIMIT - 4);

    assert_eq!(super::service_slots(None), CREDIT_REQUEST_LIMIT);
    assert_eq!(super::service_slots(Some(12)), 16);
    assert_eq!(super::service_slots(Some(1000)), CREDIT_REQUEST_LIMIT);
    let limited = ServerFeatures::parse(&format!(
        "{} worker",
        super::server_version_with(true, super::service_slots(Some(12)))
    ));
    assert_eq!(limited.transfer_slots(), 12);
    assert!(limited.batches());

    let old_agent = ServerFeatures::parse("0.1.0+sync-links-v1");
    assert!(old_agent.link_aware_hash && !old_agent.credit && !old_agent.batches());
    assert_eq!(old_agent.transfer_slots(), 6);
    let old_service = ServerFeatures::parse("0.5.1+sync-links-v1 worker");
    assert_eq!(old_service.transfer_slots(), 12);
    assert_eq!(ServerFeatures::parse("test"), ServerFeatures::default());
}

#[test]
fn transfer_engine_task_busy_marker_roundtrips() {
    let text = super::busy_message(Some(Duration::from_millis(250)), "langsamer");
    assert_eq!(
        parse_busy(&text),
        Some((Some(Duration::from_millis(250)), "langsamer"))
    );
    assert_eq!(
        parse_busy(&super::busy_message(None, "voll")),
        Some((None, "voll"))
    );
    assert!(parse_busy("too many concurrent agent requests").is_some());
    assert!(parse_busy("Permission denied").is_none());
}

#[test]
fn transfer_engine_task_batch_headers_split_by_encoded_size() {
    let entry = BatchEntry {
        path: "p".repeat(1000),
        size: 1,
        nonce: 0,
        mtime_ms: None,
    };
    let frame = Frame::BatchPut {
        entries: vec![entry.clone(), entry.clone()],
    };
    assert_eq!(frame.wire_len().unwrap(), 13 + 2 * put_entry_len(&entry));
    let item = BatchItem {
        path: "q".repeat(10),
        id: Some("id".into()),
        size: 5,
    };
    let frame = Frame::BatchGet {
        items: vec![item.clone()],
    };
    assert_eq!(frame.wire_len().unwrap(), 13 + get_item_len(&item));

    let header = vec![put_entry_len(&entry); 600];
    let sizes = vec![1u64; 600];
    let ranges = split_batch(&header, &sizes);
    assert_eq!(ranges.first().map(|range| range.start), Some(0));
    assert_eq!(ranges.last().map(|range| range.end), Some(600));
    for range in &ranges {
        assert!(range.len() <= BATCH_MAX_FILES);
        assert!(13 + header[range.clone()].iter().sum::<usize>() <= BATCH_HEADER_MAX);
    }
    let ranges = split_batch(&[30; 4], &[BATCH_MAX_BYTES / 2 + 1; 4]);
    assert_eq!(ranges, vec![0..1, 1..2, 2..3, 3..4]);
    let ranges = split_batch(&[30; 300], &[1; 300]);
    assert_eq!(ranges, vec![0..BATCH_MAX_FILES, BATCH_MAX_FILES..300]);
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_canceled_batch_upload_leaves_no_stage_behind() {
    use std::net::{Shutdown, TcpListener, TcpStream};

    let root = std::env::temp_dir().join(format!(
        "se_agent_batch_cancel_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("file.bin");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = super::serve(read, socket);
    });
    let mut client = TcpStream::connect(address).unwrap();
    let mut replies = client.try_clone().unwrap();
    write_frame(&mut client, 0, &Frame::Credit { bytes: 0 }).unwrap();
    let entries = vec![BatchEntry {
        path: destination.to_string_lossy().into_owned(),
        size: 10,
        nonce: 7,
        mtime_ms: None,
    }];
    write_frame(&mut client, 3, &Frame::BatchPut { entries }).unwrap();
    write_frame(&mut client, 3, &Frame::Data(b"half".to_vec())).unwrap();
    let stage = root.join("file.bin.se-agent-batch-0000000000000007-0.part");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stage.exists() {
        assert!(Instant::now() < deadline, "stage never appeared");
        std::thread::sleep(Duration::from_millis(5));
    }
    write_frame(&mut client, 3, &Frame::Cancel).unwrap();
    loop {
        match super::read_frame(&mut replies).unwrap() {
            Some((3, Frame::Err(_))) => break,
            Some((3, Frame::Credit { .. })) => {}
            other => panic!("unexpected reply {other:?}"),
        }
    }
    assert!(!stage.exists());
    assert!(!destination.exists());
    let _ = client.shutdown(Shutdown::Both);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
