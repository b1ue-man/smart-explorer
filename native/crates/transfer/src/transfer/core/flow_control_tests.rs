use super::super::flow_window::start_from_learned;
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
                (7..=9).contains(&control.limit()),
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
fn transfer_engine_task_flow_never_grows_without_measured_work() {
    let mut now = 0;
    let mut control = FlowControl::new(RESOURCE_CEILING, now);
    // Long operations fill the limit but neither finish nor report bytes.
    let nothing = |_: usize| 0;
    for _ in 0..20 {
        window(&mut control, &mut now, &nothing, true);
    }
    assert_eq!(control.limit(), 2);
    // Once work is measured, slow start proceeds from there.
    let linear = |limit: usize| limit as u64 * MB;
    window(&mut control, &mut now, &linear, true);
    assert_eq!(control.limit(), 4);
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

/// Deterministic multiplicative noise in [1 - spread, 1 + spread).
struct Noise(u64);

impl Noise {
    fn factor(&mut self, spread: f64) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let unit = (self.0 >> 33) as f64 / (1u64 << 31) as f64;
        1.0 + 2.0 * spread * (unit - 0.5)
    }
}

#[test]
fn transfer_engine_task_flow_holds_the_optimum_under_measurement_noise() {
    for optimum in [8usize, 24] {
        for seed in [1u64, 42] {
            let mut noise = Noise(seed);
            let mut now = 0;
            let mut control = FlowControl::new(RESOURCE_CEILING, now);
            let mut tail = Vec::new();
            for step in 0..400 {
                let factor = noise.factor(0.10);
                let goodput = (control.limit().min(optimum) as f64 * MB as f64 * factor) as u64;
                control.progress(goodput);
                now += 1_000;
                control.tick(now, true);
                if step >= 250 {
                    tail.push(control.limit());
                }
            }
            let useful: usize = tail.iter().map(|limit| (*limit).min(optimum)).sum();
            let efficiency = useful as f64 / (tail.len() * optimum) as f64;
            let mean = tail.iter().sum::<usize>() as f64 / tail.len() as f64;
            assert!(
                efficiency >= 0.9,
                "optimum {optimum}, seed {seed}: efficiency {efficiency:.3}"
            );
            assert!(
                mean <= optimum as f64 * 1.5,
                "optimum {optimum}, seed {seed}: mean limit {mean:.1} overshoots"
            );
        }
    }
}

/// `count` operations that took `latency_ms` each finish at `now`.
fn finish_ops(control: &mut FlowControl, count: u32, latency_ms: u64, now: u64) {
    for _ in 0..count {
        control.finish(OpOutcome::Done, latency_ms, now);
    }
}

#[test]
fn speed_task_flow_window_waits_for_eight_completions_or_one_second() {
    let mut control = FlowControl::new(64, 0);
    control.progress(MB);
    finish_ops(&mut control, 7, 10, 0);
    // seven completions after 300 ms and 999 ms: the window goes on
    control.tick(300, true);
    control.tick(999, true);
    assert_eq!(control.limit(), 2);
    // the eighth ends it
    finish_ops(&mut control, 1, 10, 999);
    control.tick(1_000, true);
    assert_eq!(control.limit(), 4, "slow start doubled on the full window");

    // few completions: the second ends the window
    let mut sparse = FlowControl::new(64, 0);
    sparse.progress(MB);
    finish_ops(&mut sparse, 1, 10, 0);
    sparse.tick(999, true);
    assert_eq!(sparse.limit(), 2);
    sparse.tick(1_000, true);
    assert_eq!(sparse.limit(), 4);
}

#[test]
fn speed_task_flow_window_floor_is_one_hundred_ms() {
    let mut control = FlowControl::new(64, 0);
    control.progress(MB);
    // eight operations of 1 ms: twice the latency is far below the floor
    finish_ops(&mut control, 8, 1, 10);
    control.tick(50, true);
    control.tick(99, true);
    assert_eq!(control.limit(), 2, "no window under 100 ms");
    control.tick(100, true);
    assert_eq!(control.limit(), 4);
}

#[test]
fn speed_task_flow_slow_start_needs_two_misses_to_end() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    let mut run = |control: &mut FlowControl, megabytes: u64| {
        control.progress(megabytes * MB);
        now += 1_000;
        control.tick(now, true);
    };
    run(&mut control, 2);
    assert_eq!(control.limit(), 4);
    run(&mut control, 4);
    assert_eq!(control.limit(), 8);
    // one window without +10 % measures the same limit again
    run(&mut control, 4);
    assert_eq!(control.limit(), 8, "one miss may be noise");
    // a gain afterwards resets the count and slow start goes on
    run(&mut control, 9);
    assert_eq!(control.limit(), 16);
    run(&mut control, 9);
    assert_eq!(control.limit(), 16);
    // the second miss in a row ends it at the limit before the last doubling
    run(&mut control, 9);
    assert_eq!(control.limit(), 8);
    let facts = control.take_facts();
    assert!(facts.plateau, "slow start ended");
}

#[test]
fn speed_task_flow_latency_inflation_cancels_upward_probe() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    let capped = |limit: usize| limit.min(8) as u64 * MB;
    let mut window = |control: &mut FlowControl, latency_ms: u64| {
        finish_ops(control, 8, latency_ms, now);
        control.progress(capped(control.limit()));
        now += 1_000;
        control.tick(now, true);
    };
    let mut windows = 0;
    while control.limit() <= control.settled_limit() {
        window(&mut control, 100);
        windows += 1;
        assert!(windows < 60, "no upward probe started");
    }
    let settled = control.settled_limit();
    assert!(control.limit() > settled, "an upward probe is running");
    assert!(!control.limit_inflated());
    // the probe's first window: the same goodput, operations four times slower
    window(&mut control, 400);
    assert_eq!(control.limit(), settled, "the probe was withdrawn");
    assert!(control.limit_inflated());
    assert!(control.take_facts().plateau);
}

#[test]
fn speed_task_flow_start_limit_from_hint_is_half() {
    assert_eq!(start_from_learned(16), 8);
    assert_eq!(start_from_learned(3), 2);
    assert_eq!(start_from_learned(1), 2, "never below the general start");
    assert_eq!(FlowControl::with_start(64, 0, 8).limit(), 8);
    assert_eq!(
        FlowControl::with_start(4, 0, 100).limit(),
        4,
        "ceiling wins"
    );
    assert_eq!(FlowControl::with_start(64, 0, 0).limit(), 1);
    // doubling goes on from the start
    let mut now = 0;
    let mut control = FlowControl::with_start(64, now, 8);
    let linear = |limit: usize| limit as u64 * MB;
    window(&mut control, &mut now, &linear, true);
    assert_eq!(control.limit(), 16);
}

#[test]
fn speed_task_flow_facts_saturated_and_plateau() {
    let mut control = FlowControl::new(64, 0);
    assert_eq!(control.take_facts(), FlowFacts::default());
    // 300 ms with demand filling the limit
    control.tick(300, true);
    let facts = control.take_facts();
    assert!(facts.saturated);
    assert!(!facts.plateau, "slow start has not ended");
    // a stretch shorter than a quarter second repeats the last reading
    control.tick(400, false);
    assert!(control.take_facts().saturated);
    // 600 ms without demand
    control.tick(1_000, false);
    assert!(!control.take_facts().saturated);
    // a flow at its ceiling cannot grow
    assert!(FlowControl::new(1, 0).take_facts().plateau);
}

#[test]
fn speed_task_flow_thread_refusal_halves_and_bounds() {
    let mut now = 0;
    let mut control = FlowControl::new(64, now);
    let linear = |limit: usize| limit as u64 * MB;
    for _ in 0..4 {
        window(&mut control, &mut now, &linear, true);
    }
    assert!(control.limit() >= 16);
    control.thread_refused(5, now);
    assert_eq!(control.ceiling(), 5);
    assert!(control.limit() <= 5);
    control.thread_refused(0, now);
    assert_eq!(control.ceiling(), 1, "at least one operation stays");
}
