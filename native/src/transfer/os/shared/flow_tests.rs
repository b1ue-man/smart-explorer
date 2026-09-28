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
    let pair = acquire_pair(&one, Some(&one), &cancel).expect("single permit");
    assert_eq!(one.snapshot().in_flight, 1);
    pair.finish(OpOutcome::Done);
    let other = flow(key("pair-other"), Some(1));
    let pair = acquire_pair(&one, Some(&other), &cancel).expect("two permits");
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
    let quota = io::Error::new(io::ErrorKind::QuotaExceeded, "userRateLimitExceeded");
    assert_eq!(classify_error(&quota), OpOutcome::Overload);
    let text = io::Error::other("server answered HTTP 429 Too Many Requests");
    assert_eq!(classify_error(&text), OpOutcome::Overload);
    let busy = io::Error::other("too many concurrent backend requests");
    assert_eq!(classify_error(&busy), OpOutcome::Overload);
    let plain = io::Error::new(
        io::ErrorKind::PermissionDenied,
        "IMG_4290.jpg (503 KB): permission denied",
    );
    assert_eq!(classify_error(&plain), OpOutcome::Failed);
}
