use super::SmallLane;

/// Codex 11/7: small control frames take the share the pictures leave the
/// control stream (round 12, H2: one second of burst, the window the share
/// is measured over); only what goes beyond it is booked in the pictures'
/// budget, and the share refills at its rate.
#[test]
fn review1119_task_small_frames_take_their_share() {
    let mut lane = SmallLane::default();
    // 80 kbit/s reserved: 10 000 bytes of burst.
    assert_eq!(lane.excess(0, 80_000, 9_500), 0);
    assert_eq!(lane.excess(0, 80_000, 1_000), 500);
    // 100 ms later 1000 bytes came back.
    assert_eq!(lane.excess(100_000, 80_000, 1_000), 0);
    assert_eq!(lane.excess(100_000, 80_000, 10), 10);
    // Without a reserved share every byte takes the budget.
    let mut none = SmallLane::default();
    assert_eq!(none.excess(0, 0, 40), 40);
}

/// Round 12 (H2): a frame sent once a second (`STATS`, `PONG`) whose bytes
/// make the measured control rate fits its share every time (with 200 ms of
/// burst four fifths of it took the pictures' budget for good); a look at
/// the share (`peek`, a frame that may wait) takes nothing.
#[test]
fn r12_task_periodic_frames_fit_their_share() {
    let mut lane = SmallLane::default();
    // 600 bytes a second measure 4800 bit/s.
    for second in 0..5u64 {
        assert_eq!(lane.excess(second * 1_000_000, 4_800, 600), 0, "{second}");
    }
    let mut lane = SmallLane::default();
    assert_eq!(lane.excess(0, 8_000, 1_000), 0);
    assert_eq!(lane.peek(0, 8_000, 500), 500);
    assert_eq!(lane.peek(0, 8_000, 500), 500, "a look takes nothing");
    assert_eq!(lane.excess(500_000, 8_000, 500), 0);
}
