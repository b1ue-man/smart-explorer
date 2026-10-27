use super::*;

const FRAME_US: u64 = 33_333;
const FRAME_BYTES: u64 = 25_000;

/// Sends frame `index` of a 30 frames/s stream and acknowledges it; the
/// viewer receives it `delay_us` after sending (both clocks equal here).
fn frame(rate: &mut RateController, index: u64, delay_us: u64, keyframe: bool) -> DelaySignal {
    let send_us = index * FRAME_US;
    rate.on_frame_sent(FRAME_BYTES, send_us);
    let sample = AckSample {
        send_us,
        recv_us: send_us + delay_us,
        bytes: FRAME_BYTES,
        keyframe,
    };
    rate.on_frame_acked(sample, send_us + delay_us + 10_000)
}

#[test]
fn screen_task_rate_control_backs_off_on_delay_gradient() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    assert_eq!(
        RateController::new(PathKind::Relay, 0).target_bps(),
        START_RELAY_BPS
    );

    // Constant delivery permits probing, bounded by 1.25 times the measured
    // receive rate rather than unbounded growth on a quiet screen.
    let signals: Vec<DelaySignal> = (0..60)
        .map(|index| frame(&mut rate, index, 20_000, false))
        .collect();
    assert!(signals[..MIN_DELAY_POINTS - 1]
        .iter()
        .all(|signal| *signal == DelaySignal::Learning));
    assert!(signals[MIN_DELAY_POINTS..]
        .iter()
        .all(|signal| *signal == DelaySignal::Increase));
    let grown = rate.target_bps();
    assert!(grown > START_DIRECT_BPS, "target grew to {grown}");
    assert!(
        grown <= rate.receive_rate_bps().unwrap() * 5 / 4,
        "probe must stay within measured delivery headroom"
    );

    // A queue builds up: every frame arrives 6 ms later than the one
    // before (slope 0.18). Once the window shows it, overuse holds the
    // target twice, then lowers it to 0.85 × the receive rate.
    let mut signals = Vec::new();
    for step in 0..30 {
        let signal = frame(&mut rate, 60 + step, 20_000 + (step + 1) * 6_000, false);
        signals.push(signal);
        if signal == DelaySignal::Decrease {
            break;
        }
    }
    assert_eq!(signals.last(), Some(&DelaySignal::Decrease));
    let before = &signals[signals.len() - 3..signals.len() - 1];
    assert_eq!(before, [DelaySignal::Hold, DelaySignal::Hold]);
    let receive = rate.receive_rate_bps().unwrap();
    let target = rate.target_bps();
    assert_eq!(target, (DECREASE_FACTOR * receive as f64).round() as u64);
    assert!(target < grown && receive < grown);

    // The delay window starts over after a decrease and after a key frame,
    // whose size shows as delay: neither lowers the target again at once.
    assert_eq!(frame(&mut rate, 100, 400_000, false), DelaySignal::Learning);
    assert_eq!(frame(&mut rate, 101, 900_000, true), DelaySignal::Learning);
    assert_eq!(rate.target_bps(), target);

    // A move onto the relay caps the target at the relay start value.
    let mut moving = RateController::new(PathKind::Direct, 0);
    moving.set_path(PathKind::Relay);
    assert_eq!(moving.path(), PathKind::Relay);
    assert_eq!(moving.target_bps(), START_RELAY_BPS);
}

#[test]
fn screen_task_rate_control_loss_rules() {
    let mut rate = RateController::new(PathKind::Direct, 0);
    // The first reading only sets the baseline: counters are cumulative.
    assert_eq!(rate.on_loss_counters(1_000, 0), None);
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    // Above 10 %: × (1 − 0.5 p).
    assert_eq!(rate.on_loss_counters(2_000, 200), Some(0.2));
    assert_eq!(rate.target_bps(), 5_400_000);
    // 2–10 %: hold.
    assert_eq!(rate.on_loss_counters(3_000, 250), Some(0.05));
    assert_eq!(rate.target_bps(), 5_400_000);
    // Below 2 %: release the loss ceiling; the delay target still bounds it.
    assert_eq!(rate.on_loss_counters(4_000, 260), Some(0.01));
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    // Both boundaries hold: exactly 10 % and exactly 2 %.
    assert_eq!(rate.on_loss_counters(5_000, 360), Some(0.1));
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    assert_eq!(rate.on_loss_counters(6_000, 380), Some(0.02));
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    // Growth from loss never passes the delay-based target.
    for second in 0..10 {
        rate.on_loss_counters(7_000 + second * 1_000, 380);
    }
    assert_eq!(rate.target_bps(), START_DIRECT_BPS);
    // Counters that go backwards or do not move change nothing.
    let before = rate.target_bps();
    assert_eq!(rate.on_loss_counters(100, 0), None);
    assert_eq!(rate.on_loss_counters(100, 0), None);
    assert_eq!(rate.target_bps(), before);
    // Total loss halves the target every second, down to 150 kbit/s.
    for second in 1..=40 {
        rate.on_loss_counters(100 + second * 1_000, second * 1_000);
    }
    assert_eq!(rate.target_bps(), MIN_BITRATE_BPS);
    // More lost than sent counts as total loss.
    assert_eq!(rate.on_loss_counters(50_000, 90_000), Some(1.0));
}

#[test]
fn screen_task_rate_control_quality_ladder() {
    let source = FrameSize::new(1920, 1080);
    // 6 Mbit/s carries 1080p at the full 30 frames/s.
    assert_eq!(
        quality_for(6_000_000, source, 30),
        Quality {
            fps: 30,
            size: source
        }
    );
    // Lower targets lower cadence while preserving every source pixel.
    assert_eq!(quality_for(2_000_000, source, 30).size, source);
    assert_eq!(quality_for(2_000_000, source, 30).fps, 19);
    let low = quality_for(500_000, source, 30);
    assert_eq!(low.fps, 4);
    assert_eq!(low.size, source);
    assert_eq!(quality_for(MIN_BITRATE_BPS, source, 30).size, source);
    let bottom = quality_for(10_000, source, 30).size;
    assert_eq!(bottom, source);
    assert_eq!(quality_for(10_000, source, 30).fps, 1);
    let small_source = FrameSize::new(300, 200);
    assert_eq!(quality_for(10_000, small_source, 30).size, small_source);
    // Up only with 25 % headroom and by at least 25 %, down by at least 25 %.
    let current = quality_for(2_000_000, source, 30);
    assert_eq!(adjust_quality(current, 2_200_000, source, 30), current);
    assert_eq!(adjust_quality(current, 2_600_000, source, 30), current);
    assert_eq!(adjust_quality(current, 3_200_000, source, 30).fps, 24);
    assert_eq!(adjust_quality(current, 1_800_000, source, 30), current);
    assert_eq!(adjust_quality(current, 1_500_000, source, 30).fps, 14);
    // A smaller source replaces a larger current size.
    let small = FrameSize::new(1280, 720);
    assert_eq!(adjust_quality(current, 2_000_000, small, 30).size, small);
}

/// Each cadence change restarts the encoder (key frame, new picture group,
/// new input reference). A target that ramps up in 5 % steps from 2 to
/// 8 Mbit/s moved 1080p through 19, 20, 21, 22, 23, 25, 26, 27, 29, 30
/// frames/s (nine restarts); now only through 19, 25, 30. A target that
/// swings ±8 % changes nothing, and the highest rate is still reached.
#[test]
fn screen_metadata_runtime_task_cadence_changes_only_in_coarse_steps() {
    let source = FrameSize::new(1920, 1080);
    let mut current = quality_for(2_000_000, source, 30);
    let mut seen = vec![current.fps];
    let mut target = 2_000_000.0_f64;
    while target < 8_000_000.0 {
        target *= 1.05;
        let next = adjust_quality(current, target as u64, source, 30);
        if next != current {
            seen.push(next.fps);
        }
        current = next;
    }
    assert_eq!(seen, vec![19, 25, 30]);
    let steady = quality_for(2_500_000, source, 30);
    for swing in [0.92, 1.08, 0.92, 1.08, 0.92, 1.08] {
        let target = (2_500_000.0 * swing) as u64;
        assert_eq!(adjust_quality(steady, target, source, 30), steady);
    }
    let full = Quality {
        fps: 30,
        size: source,
    };
    assert_eq!(adjust_quality(full, 2_900_000, source, 30), full);
    assert_eq!(adjust_quality(full, 2_400_000, source, 30).fps, 23);
    let near = Quality {
        fps: 27,
        size: source,
    };
    assert_eq!(adjust_quality(near, 10_000_000, source, 30).fps, 30);
}
