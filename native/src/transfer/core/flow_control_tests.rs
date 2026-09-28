use super::*;

const MB: u64 = 1024 * 1024;

/// One measurement window of 1 s with `goodput(limit)` bytes of work.
fn window(control: &mut FlowControl, now: &mut u64, goodput: &dyn Fn(usize) -> u64, full: bool) {
    control.progress(goodput(control.limit()));
    *now += 1_000;
    control.tick(*now, full);
}

#[test]
fn transfer_engine_task_flow_doubles_while_goodput_scales_up_to_ceiling() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    assert_eq!(control.limit(), 2);
    let linear = |limit: usize| limit as u64 * MB;
    for _ in 0..8 {
        window(&mut control, &mut now, &linear, true);
    }
    assert_eq!(control.limit(), 64);
}

#[test]
fn transfer_engine_task_flow_settles_where_more_concurrency_stops_paying() {
    let mut now = 0;
    let mut control = FlowControl::new(RESOURCE_CEILING, now);
    let capped = |limit: usize| limit.min(8) as u64 * MB;
    let mut highest = 0;
    for step in 0..60 {
        window(&mut control, &mut now, &capped, true);
        highest = highest.max(control.limit());
        if step > 10 {
            assert!(
                (7..=10).contains(&control.limit()),
                "limit {} left the optimum",
                control.limit()
            );
        }
    }
    assert!(highest <= 16, "slow start overshot to {highest}");
}

#[test]
fn transfer_engine_task_flow_backs_off_when_parallelism_hurts() {
    // A spinning disk: two concurrent streams are best, more seek-thrash.
    let mut now = 0;
    let mut control = FlowControl::new(RESOURCE_CEILING, now);
    let thrash = |limit: usize| {
        if limit <= 2 {
            limit as u64 * 10 * MB
        } else {
            (20 * MB).saturating_sub((limit as u64 - 2) * 2 * MB)
        }
    };
    for _ in 0..30 {
        window(&mut control, &mut now, &thrash, true);
    }
    assert!(
        control.limit() <= 3,
        "limit {} kept thrashing",
        control.limit()
    );
}

#[test]
fn transfer_engine_task_flow_overload_halves_once_per_window() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    let linear = |limit: usize| limit as u64 * MB;
    for _ in 0..4 {
        window(&mut control, &mut now, &linear, true);
    }
    let before = control.limit();
    assert!(before >= 16);
    control.finish(OpOutcome::Overload, 100, now);
    control.finish(OpOutcome::Overload, 100, now);
    control.finish(OpOutcome::Overload, 100, now);
    assert_eq!(control.limit(), before / 2);
    // Held for a few windows, then a new congestion event halves again.
    for _ in 0..4 {
        window(&mut control, &mut now, &linear, true);
    }
    let held = control.limit();
    control.finish(OpOutcome::Overload, 100, now + 1);
    assert_eq!(control.limit(), (held / 2).max(1));
}

#[test]
fn transfer_engine_task_flow_keeps_limit_without_demand() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    let linear = |limit: usize| limit as u64 * MB;
    for _ in 0..10 {
        window(&mut control, &mut now, &linear, false);
    }
    assert_eq!(control.limit(), 2);
}

#[test]
fn transfer_engine_task_flow_respects_protocol_ceiling() {
    let mut now = 0;
    let mut control = FlowControl::new(1, now);
    let linear = |limit: usize| limit as u64 * MB;
    for _ in 0..10 {
        window(&mut control, &mut now, &linear, true);
        assert_eq!(control.limit(), 1);
    }
    let mut control = FlowControl::new(64, now);
    for _ in 0..10 {
        window(&mut control, &mut now, &linear, true);
    }
    control.set_ceiling(5);
    assert_eq!(control.limit(), 5);
    assert_eq!(control.ceiling(), 5);
}
