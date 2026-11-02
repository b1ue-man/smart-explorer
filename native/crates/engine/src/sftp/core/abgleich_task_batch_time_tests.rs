//! Packets give every file the time of its source (review W4): SFTP sets it
//! on the open handle between the WRITEs and the CLOSE of the same wave, so
//! the packet costs no round trip more; a file without a time keeps the time
//! of its upload.
use super::batch_fake::{drive, entries, payload, FakeWire, Op, Sim};
use super::batch_put::put_all;
use crate::vfs::BatchPutOutcome;
use std::io::Cursor;
use std::rc::Rc;

fn run(times: &[Option<i64>]) -> (Rc<Sim>, FakeWire, Vec<String>) {
    let sim = Rc::new(Sim::default());
    let wire = FakeWire::new(&sim);
    let (mut list, data) = entries(times.len(), 2048);
    for (entry, time) in list.iter_mut().zip(times) {
        entry.mtime_ms = *time;
    }
    let outcomes = drive(&sim, put_all(&wire, &list, &mut Cursor::new(data)));
    let paths = outcomes
        .iter()
        .map(|outcome| match outcome {
            BatchPutOutcome::Published(path) => path.clone(),
            BatchPutOutcome::Failed(error) => panic!("failed: {error}"),
        })
        .collect();
    (sim, wire, paths)
}

#[test]
fn abgleich_task_sftp_packet_files_carry_the_time_of_their_source() {
    let times = [Some(1_700_000_000_123), None, Some(1_600_000_000_999)];
    let (_sim, wire, paths) = run(&times);
    assert_eq!(wire.time_of(&paths[0]), Some(1_700_000_000));
    assert_eq!(
        wire.time_of(&paths[1]),
        None,
        "no time: the upload's time stays"
    );
    assert_eq!(wire.time_of(&paths[2]), Some(1_600_000_000));
    assert_eq!(wire.content(&paths[0]), Some(payload(0, 2048)));
    assert_eq!(wire.count(Op::SetTime), 2);
    assert!(wire.stages().is_empty());
}

#[test]
fn abgleich_task_sftp_packet_time_is_set_between_the_writes_and_the_close_without_a_round_trip_more(
) {
    let timed = [Some(1_700_000_000_000); 8];
    let (with, wire, _paths) = run(&timed);
    let (without, _plain, _names) = run(&[None; 8]);
    assert_eq!(
        with.now.get(),
        without.now.get(),
        "the time rides in the wave of the WRITEs and the CLOSE"
    );
    // For every stage: its WRITE, then its time, then its CLOSE.
    let requests = wire.fs.borrow().requests.clone();
    let stages: Vec<String> = requests
        .iter()
        .filter(|(op, _)| *op == Op::SetTime)
        .map(|(_, path)| path.clone())
        .collect();
    assert_eq!(stages.len(), 8);
    for stage in stages {
        let at = |wanted: Op| {
            requests
                .iter()
                .rposition(|(op, path)| *op == wanted && *path == stage)
                .expect("request of the stage")
        };
        assert!(
            at(Op::Write) < at(Op::SetTime),
            "{stage}: time after the data"
        );
        assert!(
            at(Op::SetTime) < at(Op::Close),
            "{stage}: time before the close"
        );
    }
}
