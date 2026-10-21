use super::*;
use std::io;
use std::sync::atomic::AtomicBool;

fn key(name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("transfer-engine-task:{name}:{nanos}")
}

#[test]
fn transfer_engine_task_flow_permits_respect_the_limit() {
    let flow = flow(key("limit"), Some(1));
    let first = flow.try_acquire().expect("first permit");
    assert!(flow.try_acquire().is_none());
    assert!(!flow.has_spare());
    assert_eq!(flow.snapshot().in_flight, 1);
    first.finish(OpOutcome::Done);
    let second = flow.try_acquire().expect("permit after release");
    drop(second);
    assert_eq!(flow.snapshot().in_flight, 0);
}

#[test]
fn transfer_engine_task_flow_wait_ends_on_cancel() {
    let flow = flow(key("cancel"), Some(1));
    let _held = flow.try_acquire().expect("permit");
    let cancel = AtomicBool::new(true);
    assert!(flow.acquire(&cancel).is_none());
}

#[test]
fn transfer_engine_task_flow_waiter_wakes_when_a_permit_returns() {
    let flow = flow(key("wake"), Some(1));
    let held = flow.try_acquire().expect("permit");
    let waiter = {
        let flow = flow.clone();
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            flow.acquire(&cancel)
                .map(|permit| permit.finish(OpOutcome::Done))
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    held.finish(OpOutcome::Done);
    assert!(waiter.join().expect("waiter thread").is_some());
}

#[test]
fn transfer_engine_task_flow_pair_uses_one_permit_for_one_connection() {
    let one = flow(key("pair-one"), Some(1));
    let cancel = AtomicBool::new(false);
    let pair = acquire_pair(&one, Some(&one), 1, &cancel).expect("single permit");
    assert_eq!(one.snapshot().in_flight, 1);
    pair.finish(OpOutcome::Done);
    let other = flow(key("pair-other"), Some(1));
    let pair = acquire_pair(&one, Some(&other), 1, &cancel).expect("two permits");
    assert_eq!(one.snapshot().in_flight, 1);
    assert_eq!(other.snapshot().in_flight, 1);
    pair.progress(4096);
    drop(pair);
    assert_eq!(one.snapshot().in_flight, 0);
    assert_eq!(other.snapshot().in_flight, 0);
}

#[test]
fn transfer_engine_task_flow_registry_reuses_learned_flow() {
    let name = key("reuse");
    let first = flow(name.clone(), None);
    let second = flow(name, Some(3));
    assert!(Arc::ptr_eq(&first, &second));
    assert!(second.snapshot().limit <= 3);
}

#[test]
fn transfer_engine_task_flow_classifies_congestion_signals() {
    let congestion = crate::vfs::congestion_error("HTTP 429 rateLimitExceeded", None);
    assert_eq!(classify_error(&congestion), OpOutcome::Overload);
    let busy = io::Error::other("too many concurrent backend requests");
    assert_eq!(classify_error(&busy), OpOutcome::Overload);
    let timeout = io::Error::new(io::ErrorKind::TimedOut, "no answer");
    assert_eq!(classify_error(&timeout), OpOutcome::Overload);
    // A full disk or exhausted storage quota is permanent, not congestion.
    let quota = io::Error::new(io::ErrorKind::QuotaExceeded, "disk quota exceeded");
    assert_eq!(classify_error(&quota), OpOutcome::Failed);
    let plain = io::Error::new(
        io::ErrorKind::PermissionDenied,
        "IMG_4290.jpg (429 KB): permission denied",
    );
    assert_eq!(classify_error(&plain), OpOutcome::Failed);
}

#[test]
fn transfer_engine_task_flow_takes_turns_between_jobs() {
    let flow = flow(key("turns"), Some(1));
    let held = flow
        .acquire_for(1, &AtomicBool::new(false))
        .expect("job 1 permit");
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let spawn = |job: u64| {
        let flow = flow.clone();
        let order = order.clone();
        std::thread::spawn(move || {
            let permit = flow
                .acquire_for(job, &AtomicBool::new(false))
                .expect("permit");
            order
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(job);
            std::thread::sleep(std::time::Duration::from_millis(20));
            permit.finish(OpOutcome::Done);
        })
    };
    // Job 1 queues again before job 2 asks; job 2 must still come first.
    let again = spawn(1);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let other = spawn(2);
    std::thread::sleep(std::time::Duration::from_millis(50));
    held.finish(OpOutcome::Done);
    again.join().expect("job 1 thread");
    other.join().expect("job 2 thread");
    let order = order
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(order, vec![2, 1]);
}

#[test]
fn transfer_engine_task_flow_reserves_a_slot_for_listings() {
    let flow = flow(key("meta"), Some(1));
    let _data = flow.try_acquire().expect("data permit");
    let cancel = AtomicBool::new(false);
    let listing = flow.acquire_meta(&cancel).expect("reserved listing slot");
    assert_eq!(flow.snapshot().in_flight, 2);
    listing.finish(OpOutcome::Done);
    assert_eq!(flow.snapshot().in_flight, 1);
}

#[test]
fn transfer_engine_task_flow_pair_does_not_hoard_the_first_permit() {
    let source = flow(key("hoard-source"), Some(1));
    let target = flow(key("hoard-target"), Some(1));
    let blocker = target.try_acquire().expect("target busy");
    let waiter = {
        let source = source.clone();
        let target = target.clone();
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            acquire_pair(&source, Some(&target), 7, &cancel)
                .map(|pair| pair.finish(OpOutcome::Done))
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(150));
    // While the target is busy the source permit is not held by the waiter.
    let probe = source.try_acquire();
    assert!(probe.is_some(), "the pair hoarded the source permit");
    drop(probe);
    blocker.finish(OpOutcome::Done);
    assert!(waiter.join().expect("pair thread").is_some());
}

#[test]
fn speed_task_flow_snapshot_reports_saturated_and_plateau() {
    let flow = flow(key("facts"), Some(1));
    let idle = flow.snapshot();
    assert!(!idle.saturated);
    // a flow at its bound cannot grow
    assert!(idle.plateau);
    let held = flow.try_acquire().expect("permit");
    std::thread::sleep(std::time::Duration::from_millis(350));
    let busy = flow.snapshot();
    assert!(busy.saturated, "the only permit was in use all the time");
    assert_eq!(busy.in_flight, 1);
    held.finish(OpOutcome::Done);
    std::thread::sleep(std::time::Duration::from_millis(350));
    assert!(!flow.snapshot().saturated, "nothing was running");
    // a flow that can still grow is ramping, not at a plateau
    let growing = flow_with_identity(key("growing"), None, Some(64));
    assert!(!growing.snapshot().plateau);
}

#[test]
fn speed_task_flow_start_limit_comes_from_the_hint() {
    let hinted = Flow::new("k".into(), Some("identity".into()), Some(8), 64);
    assert_eq!(hinted.snapshot().limit, 8);
    let plain = Flow::new("k".into(), Some("identity".into()), None, 64);
    assert_eq!(plain.snapshot().limit, 2);
    let bounded = Flow::new("k".into(), None, Some(8), 4);
    assert_eq!(bounded.snapshot().limit, 4, "the bound wins");
}

#[test]
fn speed_task_flow_thread_refusal_bound_survives_lookups() {
    let name = key("refusal");
    let first = flow(name.clone(), Some(64));
    assert_eq!(first.snapshot().ceiling, 64);
    first.learn_ceiling(3);
    let learned = first.snapshot();
    assert_eq!(learned.ceiling, 3, "the bound is what ran");
    assert!(learned.limit <= 3);
    // the backend announces its own bound again at every lookup: the lower
    // bound that was taught stays
    let again = flow(name.clone(), Some(64));
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(again.snapshot().ceiling, 3);
    // a lower bound of the backend still wins
    assert_eq!(flow(name.clone(), Some(2)).snapshot().ceiling, 2);
    // and at least one operation may run
    first.learn_ceiling(0);
    assert_eq!(first.snapshot().ceiling, 1);
}

#[test]
fn speed_task_flow_finish_with_books_the_accounts_on_both_flows() {
    let one = flow(key("books-one"), Some(2));
    let other = flow(key("books-other"), Some(2));
    let cancel = AtomicBool::new(false);
    let pair = acquire_pair(&one, Some(&other), 1, &cancel).expect("two permits");
    let accounts = OpAccounts {
        source_ns: 5,
        target_ns: 7,
        cpu_ns: 11,
        wait_ns: 13,
    };
    pair.finish_with(OpOutcome::Done, accounts);
    assert_eq!(one.snapshot().accounts, accounts);
    assert_eq!(other.snapshot().accounts, accounts);
    // `finish` books nothing
    let plain = acquire_pair(&one, Some(&other), 1, &cancel).expect("two permits");
    plain.finish(OpOutcome::Done);
    assert_eq!(one.snapshot().accounts, accounts);
    // an abandoned pair feeds neither controller and books nothing
    let unused = acquire_pair(&one, Some(&other), 1, &cancel).expect("two permits");
    unused.abandon();
    assert_eq!(one.snapshot().accounts, accounts);
    assert_eq!(one.snapshot().in_flight, 0);
    assert_eq!(other.snapshot().in_flight, 0);
}

#[test]
fn speed_task_flow_waited_time_is_counted_per_flow() {
    let flow = flow(key("waited"), Some(1));
    let held = flow.try_acquire().expect("permit");
    let waiter = {
        let flow = flow.clone();
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            flow.acquire(&cancel)
                .map(|permit| permit.finish(OpOutcome::Done))
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    held.finish(OpOutcome::Done);
    assert!(waiter.join().expect("waiter thread").is_some());
    assert!(
        flow.snapshot().waited_ms >= 150,
        "{}",
        flow.snapshot().waited_ms
    );
}
