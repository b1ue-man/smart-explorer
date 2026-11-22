use super::SmallLane;

/// Codex 11/7: small control frames take the share the pictures leave the
/// control stream (200 ms of burst); only what goes beyond it is booked in
/// the pictures' budget, and the share refills at its rate.
#[test]
fn review1119_task_small_frames_take_their_share() {
    let mut lane = SmallLane::default();
    // 80 kbit/s reserved: 2000 bytes of burst.
    assert_eq!(lane.excess(0, 80_000, 1_500), 0);
    assert_eq!(lane.excess(0, 80_000, 1_000), 500);
    // 100 ms later 1000 bytes came back.
    assert_eq!(lane.excess(100_000, 80_000, 1_000), 0);
    assert_eq!(lane.excess(100_000, 80_000, 10), 10);
    // Without a reserved share every byte takes the budget.
    let mut none = SmallLane::default();
    assert_eq!(none.excess(0, 0, 40), 40);
}
