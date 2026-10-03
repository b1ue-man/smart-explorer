use super::*;

fn unique_id() -> String {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("random lock id");
    format!("{:016x}", u64::from_be_bytes(bytes))
}

#[test]
fn review_task_pair_lock_excludes_a_second_holder_until_dropped() {
    let id = unique_id();
    let first = PairLock::acquire(&id).expect("first holder");
    assert_eq!(first.id(), id);
    assert_eq!(
        PairLock::acquire(&id).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    let cancel = AtomicBool::new(false);
    assert_eq!(
        PairLock::acquire_wait(&id, Duration::from_millis(50), &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    cancel.store(true, Ordering::Release);
    assert_eq!(
        PairLock::acquire_wait(&id, Duration::from_secs(5), &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    drop(first);
    let again = PairLock::acquire(&id.to_ascii_uppercase()).expect("free again");
    assert_eq!(again.id(), id);
    assert_eq!(
        PairLock::acquire("not a hex id").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn review_task_pair_lock_id_ignores_the_side_order() {
    let left = crate::vfs::LocalBackend::new("/left");
    let right = crate::vfs::LocalBackend::new("/right");
    let forward = pair_lock_id(&left, "/left/a", &right, "/right/b");
    assert_eq!(forward, pair_lock_id(&right, "/right/b", &left, "/left/a"));
    assert_ne!(forward, pair_lock_id(&left, "/left/a", &right, "/right/c"));
    assert_eq!(forward.len(), 16);
}
