//! SFTP packets against the scripted channel of `batch_fake.rs`.
use super::batch_fake::{
    drive, entries, get, items, named, payload, FakeWire, Fault, Op, RecSink, Sim, RTT,
};
use super::batch_ops::{channel_or_unsupported, limits};
use super::batch_put::put_all;
use super::channel_pool::pool_usable;
use crate::vfs::{congestion_of, is_unique_stage, unpublished_of, BatchPut, BatchPutOutcome};
use std::io::{self, Cursor, Read};
use std::rc::Rc;

fn put(sim: &Rc<Sim>, wire: &FakeWire, list: &[BatchPut], data: Vec<u8>) -> Vec<BatchPutOutcome> {
    drive(sim, put_all(wire, list, &mut Cursor::new(data)))
}

fn published(outcome: &BatchPutOutcome) -> &str {
    match outcome {
        BatchPutOutcome::Published(path) => path,
        BatchPutOutcome::Failed(error) => panic!("failed: {error}"),
    }
}

fn failed(outcome: &BatchPutOutcome) -> &io::Error {
    match outcome {
        BatchPutOutcome::Failed(error) => error,
        BatchPutOutcome::Published(path) => panic!("published as {path}"),
    }
}

#[test]
fn speed_task_sftp_batch_put_publishes_64_files_in_few_round_trips() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    let (list, data) = entries(64, 4096);
    let outcomes = put(&sim, &wire, &list, data);
    let batched = sim.now.get();
    assert!(batched <= 5 * RTT, "{batched} ticks for 64 files");
    for (index, (entry, outcome)) in list.iter().zip(&outcomes).enumerate() {
        assert_eq!(published(outcome), entry.path);
        assert_eq!(wire.content(&entry.path), Some(payload(index, 4096)));
    }
    assert!(wire.stages().is_empty());
    assert_eq!(wire.fs.borrow().open_now, 0);
    // The same files one at a time pay every round trip of every file.
    let alone = FakeWire::new(&sim);
    let started = sim.now.get();
    for (index, entry) in list.iter().enumerate() {
        put(
            &sim,
            &alone,
            std::slice::from_ref(entry),
            payload(index, 4096),
        );
    }
    // OPEN, WRITEs with the CLOSE behind them (G3-06a), RENAME.
    assert!(sim.now.get() - started >= 3 * 64 * RTT);
}

#[test]
fn speed_task_sftp_batch_put_names_taken_are_numbered() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim)
        .file("d/a.txt", b"old a")
        .file("d/a (2).txt", b"old a2")
        .file("d/b.txt", b"old b");
    let (list, data) = named(&["d/a.txt", "d/b.txt", "d/c.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    assert_eq!(published(&outcomes[0]), "d/a (3).txt");
    assert_eq!(published(&outcomes[1]), "d/b (2).txt");
    assert_eq!(published(&outcomes[2]), "d/c.txt");
    assert_eq!(wire.content("d/a.txt"), Some(b"old a".to_vec()));
    assert_eq!(wire.content("d/a (2).txt"), Some(b"old a2".to_vec()));
    assert_eq!(wire.content("d/b.txt"), Some(b"old b".to_vec()));
    assert_eq!(wire.content("d/a (3).txt"), Some(payload(0, 10)));
}

#[test]
fn speed_task_sftp_batch_put_stage_collision_picks_new_name() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    wire.fail(
        Op::Open,
        "d/a.txt",
        Fault::Status(io::ErrorKind::AlreadyExists),
    );
    wire.fail(Op::Open, "d/a.txt", Fault::Taken);
    let (list, data) = named(&["d/a.txt", "d/b.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    assert_eq!(published(&outcomes[0]), "d/a.txt");
    assert_eq!(published(&outcomes[1]), "d/b.txt");
    let opened: Vec<String> = wire
        .fs
        .borrow()
        .requests
        .iter()
        .filter(|(op, path)| *op == Op::Open && path.starts_with("d/a.txt"))
        .map(|(_, path)| path.clone())
        .collect();
    assert_eq!(opened.len(), 3, "two collisions, then a free name");
    assert!(opened
        .iter()
        .all(|path| is_unique_stage(path.rsplit('/').next().unwrap_or(""))));
    assert_eq!(
        opened
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    // The stage someone else created is theirs: left alone, never adopted.
    assert_eq!(wire.stages().len(), 1);
    assert_eq!(wire.content(&wire.stages()[0]), Some(Vec::new()));
}

#[test]
fn speed_task_sftp_batch_put_write_failure_removes_stage_and_is_unpublished() {
    let sim = Rc::new(Sim::default());
    let mut wire = FakeWire::new(&sim);
    wire.chunk = 1024;
    wire.fail(Op::Write, "d/b.txt", Fault::Status(io::ErrorKind::Other));
    let (list, data) = named(&["d/a.txt", "d/b.txt", "d/c.txt"], 4096);
    let outcomes = put(&sim, &wire, &list, data);
    assert_eq!(published(&outcomes[0]), "d/a.txt");
    assert!(unpublished_of(failed(&outcomes[1])));
    assert_eq!(published(&outcomes[2]), "d/c.txt");
    assert!(wire.stages().is_empty(), "{:?}", wire.stages());
    assert_eq!(wire.content("d/b.txt"), None);
    assert_eq!(wire.fs.borrow().open_now, 0, "the handle is closed");
}

#[test]
fn speed_task_sftp_batch_put_lost_rename_reply_is_not_unpublished() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    wire.fail(Op::Rename, "d/a.txt", Fault::LostAfterEffect);
    let (list, data) = named(&["d/a.txt", "d/b.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    let error = failed(&outcomes[0]);
    assert!(!unpublished_of(error), "an unknown result is never labeled");
    assert!(error.to_string().contains(".se-upload-"), "{error}");
    assert_eq!(published(&outcomes[1]), "d/b.txt");
    let renames = wire
        .fs
        .borrow()
        .requests
        .iter()
        .filter(|(op, path)| *op == Op::Rename && path.starts_with("d/a.txt"))
        .count();
    assert_eq!(renames, 1, "no second attempt after an unknown result");
}

#[test]
fn speed_task_sftp_batch_put_lost_reply_before_rename_is_unpublished() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    wire.fail(Op::Write, "d/a.txt", Fault::LostAfterEffect);
    let (list, data) = named(&["d/a.txt", "d/b.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    let error = failed(&outcomes[0]);
    assert!(
        unpublished_of(error),
        "no RENAME was sent: nothing is published"
    );
    assert!(error.to_string().contains(".se-upload-"), "{error}");
    assert_eq!(published(&outcomes[1]), "d/b.txt");
    assert_eq!(wire.content("d/a.txt"), None);
    let renames = wire
        .fs
        .borrow()
        .requests
        .iter()
        .filter(|(op, path)| *op == Op::Rename && path.starts_with("d/a.txt"))
        .count();
    assert_eq!(renames, 0);
}

#[test]
fn speed_task_sftp_batch_put_destination_is_directory_is_not_replaced() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim).dir("d/a.txt");
    let (list, data) = named(&["d/a.txt", "d/b.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    assert_eq!(published(&outcomes[0]), "d/a (2).txt");
    assert!(wire
        .fs
        .borrow()
        .nodes
        .get("d/a.txt")
        .is_some_and(|node| node.dir));
    assert_eq!(wire.content("d/a (2).txt"), Some(payload(0, 10)));
}

#[test]
fn speed_task_sftp_batch_put_window_follows_open_handles() {
    let sim = Rc::new(Sim::default());
    let mut wire = FakeWire::new(&sim);
    wire.open_handles = Some(8);
    let (list, data) = entries(64, 100);
    let outcomes = put(&sim, &wire, &list, data);
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, BatchPutOutcome::Published(_))));
    assert_eq!(
        wire.fs.borrow().open_max,
        8,
        "as many as allowed, never more"
    );
    // A server that allows more than 64 still gets at most 64 at once.
    let mut wide = FakeWire::new(&sim);
    wide.open_handles = Some(1000);
    let (list, data) = entries(100, 100);
    put(&sim, &wide, &list, data);
    assert_eq!(wide.fs.borrow().open_max, 64);
}

/// `data` for `good` entries of 100 bytes, then the stream fails with `kind`.
fn put_then_fail(kind: io::ErrorKind, good: usize) -> (Rc<Sim>, FakeWire, Vec<BatchPutOutcome>) {
    struct Failing(Cursor<Vec<u8>>, io::ErrorKind);
    impl Read for Failing {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            match self.0.read(out)? {
                0 if !out.is_empty() => Err(io::Error::new(self.1, "fixture: stream failed")),
                read => Ok(read),
            }
        }
    }
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    let (list, data) = entries(5, 100);
    let mut stream = Failing(Cursor::new(data[..good * 100].to_vec()), kind);
    let outcomes = drive(&sim, put_all(&wire, &list, &mut stream));
    (sim, wire, outcomes)
}

#[test]
fn speed_task_sftp_batch_put_read_error_stops_later_members() {
    let (_, wire, outcomes) = put_then_fail(io::ErrorKind::ConnectionReset, 3);
    for outcome in &outcomes[..3] {
        assert!(matches!(outcome, BatchPutOutcome::Published(_)));
    }
    let error = failed(&outcomes[3]);
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    assert!(!unpublished_of(error));
    assert!(unpublished_of(failed(&outcomes[4])), "never started");
    assert!(wire.stages().is_empty());
}

#[test]
fn speed_task_sftp_batch_cancel() {
    // A canceled packet stream answers `Interrupted` for good: the packet
    // ends at once instead of reading again, running members finish.
    let (_, wire, outcomes) = put_then_fail(io::ErrorKind::Interrupted, 2);
    assert!(matches!(outcomes[0], BatchPutOutcome::Published(_)));
    assert!(matches!(outcomes[1], BatchPutOutcome::Published(_)));
    for outcome in &outcomes[2..] {
        assert_eq!(failed(outcome).kind(), io::ErrorKind::Interrupted);
    }
    assert!(wire.stages().is_empty());
    assert_eq!(wire.fs.borrow().open_now, 0);
}

#[test]
fn speed_task_sftp_batch_put_empty_file_sends_no_write() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    let list = vec![
        BatchPut {
            path: "d/empty.txt".to_string(),
            size: 0,
        },
        BatchPut {
            path: "d/full.txt".to_string(),
            size: 10,
        },
    ];
    let outcomes = put(&sim, &wire, &list, payload(1, 10));
    assert_eq!(published(&outcomes[0]), "d/empty.txt");
    assert_eq!(wire.content("d/empty.txt"), Some(Vec::new()));
    assert_eq!(wire.count(Op::Write), 1, "only the file with bytes writes");
}

#[test]
fn speed_task_sftp_batch_get_delivers_in_request_order() {
    let sim = Rc::new(Sim::default());
    let mut wire = FakeWire::new(&sim);
    for index in 0..6 {
        wire = wire.file(&format!("d/f{index}"), &payload(index, 50));
        // The last file answers first.
        wire.delay
            .insert(format!("d/f{index}"), (5 - index as u64) * RTT);
    }
    let mut sink = RecSink::default();
    get(&sim, &wire, &items(&[50; 6]), &mut sink).expect("packet");
    let expected: Vec<String> = (0..6)
        .flat_map(|index| [format!("begin {index}"), format!("end {index}")])
        .collect();
    assert_eq!(sink.events, expected);
    for index in 0..6 {
        assert_eq!(sink.data[&index], payload(index, 50));
        assert_eq!(sink.ends[&index], None);
    }
    assert_eq!(wire.fs.borrow().open_now, 0);
}

#[test]
fn speed_task_sftp_batch_get_detects_growth_and_short_files() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim)
        .file("d/f0", &payload(0, 10))
        .file("d/f1", &payload(1, 20))
        .file("d/f2", &payload(2, 6));
    let mut sink = RecSink::default();
    get(&sim, &wire, &items(&[10, 10, 10]), &mut sink).expect("packet");
    assert_eq!(sink.ends[&0], None);
    assert_eq!(
        sink.data[&1],
        payload(1, 20)[..10].to_vec(),
        "no byte beyond the listed size"
    );
    assert_eq!(sink.ends[&1], Some(io::ErrorKind::InvalidData), "grown");
    assert_eq!(sink.data[&2], payload(2, 6));
    assert_eq!(sink.ends[&2], Some(io::ErrorKind::InvalidData), "short");
}

#[test]
fn speed_task_sftp_batch_get_missing_file_is_failed_not_ended() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim)
        .file("d/f0", &payload(0, 10))
        .file("d/f2", &payload(2, 10));
    let mut sink = RecSink::default();
    get(&sim, &wire, &items(&[10, 10, 10]), &mut sink).expect("packet");
    assert_eq!(sink.failed[&1], io::ErrorKind::NotFound);
    assert!(!sink.ends.contains_key(&1) && !sink.data.contains_key(&1));
    assert_eq!(
        sink.events,
        ["begin 0", "end 0", "failed 1", "begin 2", "end 2"]
    );
}

#[test]
fn speed_task_sftp_batch_get_sink_error_stops_and_closes_handles() {
    let sim = Rc::new(Sim::default());
    let mut wire = FakeWire::new(&sim);
    wire.open_handles = Some(2);
    for index in 0..8 {
        wire = wire.file(&format!("d/f{index}"), &payload(index, 10));
    }
    let mut sink = RecSink {
        fail_begin_at: Some(0),
        ..RecSink::default()
    };
    let error = get(&sim, &wire, &items(&[10; 8]), &mut sink).expect_err("sink refused");
    assert!(error.to_string().contains("sink is full"));
    assert_eq!(wire.fs.borrow().open_now, 0, "every handle is closed");
    assert!(
        wire.count(Op::OpenRead) < 8,
        "nothing new starts after the error"
    );
    assert_eq!(sink.events, ["begin 0"]);
}

#[test]
fn speed_task_sftp_batch_get_rereads_answers_the_server_cut() {
    let sim = Rc::new(Sim::default());
    let mut wire = FakeWire::new(&sim)
        .file("d/f0", &payload(0, 20))
        .file("d/f1", &payload(1, 20));
    wire.chunk = 8;
    wire.cut = Some(3);
    let mut sink = RecSink::default();
    get(&sim, &wire, &items(&[20, 20]), &mut sink).expect("packet");
    assert_eq!(sink.data[&0], payload(0, 20));
    assert_eq!(sink.data[&1], payload(1, 20));
    assert_eq!((sink.ends[&0], sink.ends[&1]), (None, None));
}

#[test]
fn speed_task_sftp_batch_limits_from_open_handles() {
    assert_eq!(limits(None).max_files, 64);
    assert_eq!(limits(Some(8)).max_files, 8);
    assert_eq!(limits(Some(1000)).max_files, 64);
    assert_eq!(limits(Some(64)).max_files, 64);
    assert_eq!(limits(Some(0)).max_files, 64);
    assert_eq!(limits(Some(8)).max_bytes, 16 * 1024 * 1024);
}

#[test]
fn speed_task_sftp_batch_unavailable_pool_is_unsupported() {
    let error = channel_or_unsupported::<u8>(None).expect_err("no channel");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert_eq!(channel_or_unsupported(Some(5)).expect("a channel"), 5);
    assert!(
        !pool_usable(true, 2, None),
        "switched off for this connection"
    );
    assert!(
        !pool_usable(false, 0, Some(0)),
        "the server left no room for a channel"
    );
    assert!(pool_usable(false, 0, None) && pool_usable(false, 1, Some(0)));
}

#[test]
fn speed_task_sftp_batch_open_failure_is_congestion() {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    // A plain failure status (no taken name behind it), a lost reply, and a
    // specific status: only the specific one stays a failure of the file.
    wire.fail(Op::Open, "d/a.txt", Fault::Status(io::ErrorKind::Other));
    wire.fail(Op::Open, "d/b.txt", Fault::LostAfterEffect);
    wire.fail(
        Op::Open,
        "d/c.txt",
        Fault::Status(io::ErrorKind::PermissionDenied),
    );
    let (list, data) = named(&["d/a.txt", "d/b.txt", "d/c.txt", "d/d.txt"], 10);
    let outcomes = put(&sim, &wire, &list, data);
    for outcome in &outcomes[..2] {
        let error = failed(outcome);
        assert!(congestion_of(error).is_some(), "{error}");
        assert!(!unpublished_of(error));
    }
    let denied = failed(&outcomes[2]);
    assert!(congestion_of(denied).is_none() && unpublished_of(denied));
    assert_eq!(published(&outcomes[3]), "d/d.txt");
    // The same on a read: overload, but a missing file is its own failure.
    let read = FakeWire::new(&sim).file("d/f1", &payload(1, 10));
    read.fail(Op::OpenRead, "d/f0", Fault::Status(io::ErrorKind::Other));
    let mut sink = RecSink::default();
    get(&sim, &read, &items(&[10, 10, 10]), &mut sink).expect("packet");
    assert_eq!(sink.congested, [0]);
    assert_eq!(sink.failed[&2], io::ErrorKind::NotFound);
}

#[test]
fn speed_task_sftp_batch_closes_every_handle_on_cancel() {
    // A canceled stream: every handle opened is closed.
    let (_, wire, _) = put_then_fail(io::ErrorKind::Interrupted, 3);
    assert_eq!(wire.count(Op::Open), wire.count(Op::Close));
    assert_eq!(wire.fs.borrow().open_now, 0);
    // A failed write: its handle is closed as well.
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    wire.fail(Op::Write, "d/b.txt", Fault::Status(io::ErrorKind::Other));
    let (list, data) = named(&["d/a.txt", "d/b.txt"], 10);
    put(&sim, &wire, &list, data);
    assert_eq!(wire.count(Op::Open), wire.count(Op::Close));
    // A sink that stops the download: nothing stays open.
    let mut read = FakeWire::new(&sim);
    read.open_handles = Some(2);
    for index in 0..6 {
        read = read.file(&format!("d/f{index}"), &payload(index, 10));
    }
    let mut sink = RecSink {
        fail_begin_at: Some(1),
        ..RecSink::default()
    };
    get(&sim, &read, &items(&[10; 6]), &mut sink).expect_err("sink refused");
    assert_eq!(read.count(Op::OpenRead), read.count(Op::Close));
    assert_eq!(read.fs.borrow().open_now, 0);
}
