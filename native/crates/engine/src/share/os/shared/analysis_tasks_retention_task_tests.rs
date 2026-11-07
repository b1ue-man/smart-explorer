//! R-FS-4 (plan 2026-11-07): one device keeps at most four retained analysis
//! tasks; the room for another comes from its own oldest finished result
//! nobody is attached to, never from a running or attached task and never
//! from another device.
use super::super::{Failure, Work};
use super::*;
use crate::{
    analytics::{Progress, ReclaimProgress},
    share::{
        analysis_resources::Reservation, fs::ShareExportConfig, fs_access::FsAccess,
        wire::FsStorageAnalysis,
    },
};
use std::{sync::atomic::AtomicU32, sync::Mutex, time::Duration};

/// What a retained task is doing.
enum State {
    /// Still analysing, nobody attached.
    Running,
    /// Still analysing, a client is attached.
    Attached,
    /// Finished; the last client left at this time.
    Done(Instant),
}

fn principal(device: &str) -> PeerPrincipal {
    PeerPrincipal::new(
        "direct",
        "lookup",
        device,
        format!("key-{device}"),
        format!("node-{device}"),
    )
}

fn task(state: State) -> Arc<Task> {
    let progress = Progress::default();
    let reclaim = ReclaimProgress {
        cancel: progress.cancel.clone(),
        ..Default::default()
    };
    let done = Some(Err(Failure {
        kind: io::ErrorKind::Other,
        text: "fertig".into(),
    }));
    let (result, attached) = match state {
        State::Running => (None, (0, Instant::now(), 1)),
        State::Attached => (None, (1, Instant::now(), 1)),
        State::Done(at) => (done, (0, at, 1)),
    };
    Arc::new(Task {
        work: Work::Analysis(FsStorageAnalysis::default()),
        binding: String::new(),
        progress,
        reclaim,
        _authority: FsAccess::dynamic(ShareExportConfig::default()),
        _hold: Reservation::metadata(0).unwrap(),
        queue: AtomicU32::new(0),
        result: Mutex::new(result),
        attached: Mutex::new(attached),
    })
}

fn insert(
    table: &mut HashMap<Key, Arc<Task>>,
    principal: &PeerPrincipal,
    id: &str,
    state: State,
) -> Arc<Task> {
    let task = task(state);
    table.insert((principal.clone(), id.to_owned()), task.clone());
    task
}

fn ids(table: &HashMap<Key, Arc<Task>>, principal: &PeerPrincipal) -> Vec<String> {
    let mut ids: Vec<String> = table
        .keys()
        .filter(|key| &key.0 == principal)
        .map(|key| key.1.clone())
        .collect();
    ids.sort();
    ids
}

/// T-R-FS-4: four finished, detached results of one device: the next task
/// takes the place of the one nobody asked for the longest.
#[test]
fn review_task_analysis_retention_drops_the_oldest_finished_result_of_a_full_device() {
    let (a, b) = (principal("a"), principal("b"));
    let start = Instant::now();
    let at = |seconds| start + Duration::from_secs(seconds);
    let mut table = HashMap::new();
    for (id, seconds) in [("r0", 30), ("r1", 10), ("r2", 0), ("r3", 20)] {
        insert(&mut table, &a, id, State::Done(at(seconds)));
    }
    let kept = insert(&mut table, &b, "s0", State::Done(at(0)));
    let oldest = table[&(a.clone(), "r2".to_string())].clone();

    make_room(&mut table, &a).unwrap();
    assert_eq!(ids(&table, &a), ["r0", "r1", "r3"]);
    // Its spool is released at once, not after the ten minutes of grace.
    let result = oldest.result.lock().unwrap();
    assert!(matches!(&*result, Some(Err(failure)) if failure.text == "Analyseergebnis verworfen"));
    drop(result);
    // Three tasks leave room for a fourth; nothing more goes.
    make_room(&mut table, &a).unwrap();
    assert_eq!(ids(&table, &a), ["r0", "r1", "r3"]);
    // Another device keeps its result, and a device without tasks has room.
    assert_eq!(ids(&table, &b), ["s0"]);
    assert!(Arc::ptr_eq(&table[&(b.clone(), "s0".to_string())], &kept));
    make_room(&mut table, &principal("c")).unwrap();
    make_room(&mut table, &b).unwrap();
    assert_eq!(table.len(), 4);
}

/// T-R-FS-4: a device whose four tasks all run or have a client attached is
/// told to retry later and loses none of them; a finished detached one among
/// them is the room; other devices' results are never the room.
#[test]
fn review_task_analysis_retention_refuses_a_full_device_without_discarding_live_tasks() {
    let (a, b) = (principal("a"), principal("b"));
    let start = Instant::now();
    let mut table = HashMap::new();
    insert(&mut table, &a, "r0", State::Running);
    insert(&mut table, &a, "r1", State::Running);
    insert(&mut table, &a, "r2", State::Attached);
    // Finished but a client is still attached: not to be discarded.
    let attached = insert(&mut table, &a, "r3", State::Done(start));
    *attached.attached.lock().unwrap() = (1, start, 1);
    insert(&mut table, &b, "s0", State::Done(start));

    let error = make_room(&mut table, &a).unwrap_err();
    assert!(crate::vfs::congestion_of(&error).is_some(), "{error}");
    assert_eq!(ids(&table, &a), ["r0", "r1", "r2", "r3"]);
    assert_eq!(ids(&table, &b), ["s0"], "no room from another device");

    // The client of the finished result leaves: it is now the room.
    *attached.attached.lock().unwrap() = (0, start, 1);
    make_room(&mut table, &a).unwrap();
    assert_eq!(ids(&table, &a), ["r0", "r1", "r2"]);
}
