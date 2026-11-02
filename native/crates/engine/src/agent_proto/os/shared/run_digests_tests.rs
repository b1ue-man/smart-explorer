//! One read per file and run, however the two walks of a side interleave.
use super::*;
use std::sync::atomic::AtomicUsize;

fn key(name: &str) -> FileKey {
    (PathBuf::from(name), 3, 7)
}

#[test]
fn abgleich_task_run_digests_two_walks_read_a_file_once() {
    let run = RunDigests::default();
    let stop = AtomicBool::new(false);
    let reads = AtomicUsize::new(0);
    let read = || {
        reads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
        Ok("abc".to_string())
    };
    std::thread::scope(|scope| {
        let first = scope.spawn(|| run.once(key("f"), &stop, read));
        let second = scope.spawn(|| run.once(key("f"), &stop, read));
        assert_eq!(first.join().unwrap().unwrap(), "abc");
        assert_eq!(second.join().unwrap().unwrap(), "abc");
    });
    assert_eq!(reads.load(Ordering::SeqCst), 1, "the second walk waited");
    assert_eq!(run.held(), 0, "both walks took it: forgotten");
}

#[test]
fn abgleich_task_run_digests_a_later_walk_takes_the_stored_digest() {
    let run = RunDigests::default();
    let stop = AtomicBool::new(false);
    assert_eq!(
        run.once(key("g"), &stop, || Ok("one".into())).unwrap(),
        "one"
    );
    assert_eq!(run.held(), 1, "a file only one walk asked for stays");
    let second = run.once(key("g"), &stop, || panic!("read twice")).unwrap();
    assert_eq!(second, "one");
    assert_eq!(run.held(), 0);
}

#[test]
fn abgleich_task_run_digests_a_changed_file_is_read_again() {
    let run = RunDigests::default();
    let stop = AtomicBool::new(false);
    run.once(key("h"), &stop, || Ok("old".into())).unwrap();
    let changed: FileKey = (PathBuf::from("h"), 4, 7);
    assert_eq!(
        run.once(changed, &stop, || Ok("new".into())).unwrap(),
        "new"
    );
}

#[test]
fn abgleich_task_run_digests_a_failed_read_leaves_the_file_to_the_waiting_walk() {
    let run = RunDigests::default();
    let stop = AtomicBool::new(false);
    let failed = run.once(key("i"), &stop, || Err(io::Error::other("unreadable")));
    assert!(failed.is_err());
    assert_eq!(run.held(), 0, "no claim is left behind");
    assert_eq!(run.once(key("i"), &stop, || Ok("ok".into())).unwrap(), "ok");
}

#[test]
fn abgleich_task_run_digests_a_waiting_walk_stops_when_canceled() {
    let run = RunDigests::default();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            run.once(key("j"), &stop, || {
                std::thread::sleep(Duration::from_millis(300));
                Ok("late".into())
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        stop.store(true, Ordering::Relaxed);
        let waiter = run.once(key("j"), &stop, || Ok("never".into()));
        assert_eq!(waiter.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(reader.join().unwrap().unwrap(), "late");
    });
}
