//! Transfer-engine protocol additions: new frames, credit accounting,
//! capability labels and batch bounds.
use super::credit::encoded_cost;
use super::{
    credit_cost, get_item_len, has_link_aware_hash, parse_busy, put_entry_len, server_version,
    split_batch, window_target, write_frame, BatchEntry, BatchItem, Frame, RecvWindow, SendCredit,
    ServerFeatures, StreamCount, BATCH_HEADER_MAX, BATCH_MAX_BYTES, BATCH_MAX_FILES, CHUNK,
    CREDIT_CONNECTION_BUDGET, CREDIT_INITIAL, CREDIT_REQUEST_LIMIT, CREDIT_WINDOW_MAX,
};
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn new_frames() -> Vec<Frame> {
    vec![
        Frame::Credit { bytes: 1 << 40 },
        Frame::BatchPut {
            entries: vec![
                BatchEntry {
                    path: "/a/ä.txt".into(),
                    size: 0,
                    nonce: u64::MAX,
                },
                BatchEntry {
                    path: "/b".into(),
                    size: 7,
                    nonce: 1,
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
