use super::*;

#[test]
fn abgleich_task_creation_time_is_offered_only_while_the_guard_lives() {
    assert_eq!(offered(), None);
    {
        let _guard = CreationTime::offer(1_700_000_000_123);
        assert_eq!(offered(), Some(1_700_000_000_123));
    }
    assert_eq!(offered(), None, "nothing outlives the open");
}

#[test]
fn abgleich_task_creation_time_unknown_offers_nothing_and_nests() {
    let outer = CreationTime::offer(5_000);
    {
        let _unknown = CreationTime::offer(0);
        assert_eq!(offered(), None, "an unknown time hides the outer offer");
    }
    assert_eq!(offered(), Some(5_000), "the outer offer is restored");
    drop(outer);
    assert_eq!(offered(), None);
}

#[test]
fn abgleich_task_creation_time_is_private_to_its_thread() {
    let _guard = CreationTime::offer(9_000);
    let other = std::thread::spawn(offered).join().unwrap();
    assert_eq!(other, None);
}
