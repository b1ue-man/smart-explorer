use super::*;
use crate::share::session::PeerPrincipal;

#[test]
fn review_task_host_fairness_borrows_capacity_and_prioritizes_waiting_device() {
    let pool = Pool::new(4);
    let mut owned = Vec::new();
    for _ in 0..4 {
        owned.push(pool.enqueue("a").unwrap().try_acquire().unwrap().unwrap());
    }
    let same = pool.enqueue("a").unwrap();
    let other = pool.enqueue("b").unwrap();
    assert!(other.try_acquire().unwrap().is_none());
    drop(owned.pop());
    assert!(same.try_acquire().unwrap().is_none());
    let foreign = other.try_acquire().unwrap().unwrap();
    assert_eq!(pool.state.lock().unwrap().used, 4);
    drop(foreign);
    drop(owned);
    assert!(same.try_acquire().unwrap().is_some());
}

#[test]
fn review_task_host_fairness_shares_direct_room_device_but_keeps_principal() {
    let direct = PeerPrincipal::new("direct", "lookup", "device-one", "key", "node");
    let room = PeerPrincipal::new("room", "room-one", "device-alias", "key", "node");
    assert_ne!(direct, room);
    assert_eq!(direct.device_identity(), room.device_identity());
    let pool = Pool::new(1);
    let held = pool
        .enqueue(direct.device_identity())
        .unwrap()
        .try_acquire()
        .unwrap()
        .unwrap();
    let waiting = pool.enqueue(room.device_identity()).unwrap();
    assert!(waiting.try_acquire().unwrap().is_none());
    drop(held);
    assert!(waiting.try_acquire().unwrap().is_some());
}

#[test]
fn review_task_host_fairness_cancelled_queue_is_reusable_and_bounded() {
    let pool = Pool::new(1);
    let held = pool.enqueue("a").unwrap().try_acquire().unwrap().unwrap();
    let first = pool.enqueue("a").unwrap();
    let second = pool.enqueue("a").unwrap();
    assert!(matches!(pool.enqueue("a"), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    let other = pool.enqueue("b").unwrap();
    drop(first);
    drop(second);
    drop(held);
    assert!(other.try_acquire().unwrap().is_some());
    assert_eq!(pool.state.lock().unwrap().used, 0);
}
