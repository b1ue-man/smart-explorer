use super::ResetSchedule;
use std::collections::HashSet;
use std::time::{Duration, Instant};

fn ordered_ids(schedule: &ResetSchedule) -> Vec<u64> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = schedule.first;
    let mut previous = None;
    let mut deadline = None;
    while let Some(id) = cursor {
        assert!(seen.insert(id), "reset links must not form a cycle");
        let links = schedule.pending.get(&id).expect("linked request must exist");
        assert_eq!(links.previous, previous);
        if let Some(earlier) = deadline { assert!(links.deadline >= earlier); }
        deadline = Some(links.deadline);
        ids.push(id);
        previous = Some(id);
        cursor = links.next;
    }
    assert_eq!(previous, schedule.last);
    assert_eq!(ids.len(), schedule.pending.len(), "no unlinked pending tickets");
    ids
}

#[test]
fn mount_vault_task_reset_schedule_ten_thousand_rearm_and_drain() {
    let mut schedule = ResetSchedule::default();
    let base = Instant::now();
    let expected = (1..=10_000u64).collect::<Vec<_>>();
    for &id in &expected {
        assert_eq!(schedule.arm(id, base + Duration::from_micros(id)).unwrap(), id == 1);
    }
    assert_eq!(ordered_ids(&schedule), expected);
    for &id in &expected {
        let (front, old_deadline) = schedule.front().unwrap();
        assert_eq!(front, id);
        assert!(schedule.remove(id));
        assert!(!schedule.arm(id, base + Duration::from_secs(60)
            + Duration::from_micros(id)).unwrap());
        assert!(schedule.pending[&id].deadline > old_deadline);
    }
    assert_eq!(ordered_ids(&schedule), expected);
    for &id in expected.iter().rev() { schedule.remove(id); }
    assert!(ordered_ids(&schedule).is_empty());
    assert!(schedule.front().is_none());
    assert!(schedule.pending.capacity() <= 32);
}

#[test]
fn mount_vault_task_reset_schedule_unlinks_and_clamps_deadlines() {
    let mut schedule = ResetSchedule::default();
    let base = Instant::now();
    for id in [11, 22, 33, 44, 55] {
        schedule.arm(id, base + Duration::from_millis(id)).unwrap();
    }
    assert!(!schedule.remove(33)); // Middle.
    assert!(!schedule.remove(55)); // Tail.
    assert!(schedule.remove(11)); // Head.
    assert!(!schedule.remove(11)); // Already removed.
    assert_eq!(ordered_ids(&schedule), [22, 44]);
    let retained_deadline = schedule.last_deadline.unwrap();
    schedule.arm(33, base).unwrap();
    assert!(schedule.pending[&33].deadline >= retained_deadline);
    schedule.arm(55, base + Duration::from_secs(5)).unwrap();
    schedule.arm(11, base).unwrap();
    assert_eq!(ordered_ids(&schedule), [22, 44, 33, 55, 11]);
    let before = ordered_ids(&schedule);
    assert!(schedule.arm(22, base).is_err());
    assert_eq!(ordered_ids(&schedule), before);
    let last_deadline = schedule.last_deadline.unwrap();
    for id in before { schedule.remove(id); }
    assert!(schedule.arm(77, base).unwrap());
    assert_eq!(schedule.front(), Some((77, last_deadline)));
    assert_eq!(ordered_ids(&schedule), [77]);
    assert!(schedule.remove(77));
    assert!(ordered_ids(&schedule).is_empty());
}
