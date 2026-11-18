use super::*;

const FRAME: u64 = FRAME_US as u64;

/// Pushes frames `range` as a steady stream: frame n captured at n·5 ms and
/// arriving `delay` µs later.
fn steady(buffer: &mut JitterBuffer<u32>, range: std::ops::Range<u32>, delay: u64) {
    for sequence in range {
        let host = u64::from(sequence) * FRAME;
        buffer.push(sequence, host, host + delay, sequence);
    }
}

#[test]
fn audio_task_jitter_steady_stream_plays_in_order_with_one_frame_of_delay() {
    let mut buffer = JitterBuffer::default();
    assert_eq!(buffer.pop(), Playout::Silence);
    steady(&mut buffer, 0..1, 3_000);
    // No jitter measured: one frame is enough to start.
    assert_eq!(buffer.target(), MIN_TARGET);
    assert_eq!(buffer.pop(), Playout::Frame(0));
    for sequence in 1..50 {
        steady(&mut buffer, sequence..sequence + 1, 3_000);
        assert_eq!(buffer.pop(), Playout::Frame(sequence));
    }
    assert_eq!(buffer.jitter_us(), 0.0);
    // Running dry is silence and counts once.
    assert_eq!(buffer.pop(), Playout::Silence);
    assert_eq!(buffer.stats().underruns, 1);
    assert_eq!(buffer.stats().received, 50);
}

#[test]
fn audio_task_jitter_reorders_conceals_gaps_and_drops_late_frames() {
    let mut buffer = JitterBuffer::default();
    // 2 arrives before 1; 3 is lost.
    for sequence in [0u32, 2, 1, 4, 5] {
        let host = u64::from(sequence) * FRAME;
        buffer.push(sequence, host, host + 1_000, sequence);
    }
    let played: Vec<_> = (0..6).map(|_| buffer.pop()).collect();
    assert_eq!(
        played,
        vec![
            Playout::Frame(0),
            Playout::Frame(1),
            Playout::Frame(2),
            Playout::Lost,
            Playout::Frame(4),
            Playout::Frame(5),
        ]
    );
    // 3 arrives after its turn: late, dropped, never played.
    buffer.push(3, 3 * FRAME, 40 * FRAME, 3);
    assert_eq!(buffer.stats().late, 1);
    assert_eq!(buffer.stats().lost, 1);
    assert_eq!(buffer.buffered(), 0);
}

#[test]
fn audio_task_jitter_target_follows_measured_jitter_within_its_bounds() {
    let mut buffer = JitterBuffer::default();
    // Arrivals alternate 0 and 10 ms behind their capture: |D| = 10 ms.
    for sequence in 0..400u32 {
        let host = u64::from(sequence) * FRAME;
        let delay = if sequence % 2 == 0 { 0 } else { 10_000 };
        buffer.push(sequence, host, host + delay, sequence);
        let _ = buffer.pop();
    }
    // J converges to 10 ms: ceil(2 · 10 / 5) = 4 frames.
    assert!((buffer.jitter_us() - 10_000.0).abs() < 100.0);
    assert_eq!(buffer.target(), 4);
    // The report names depth and target in µs and the counters.
    let report = buffer.report(5_300);
    assert_eq!(report.target_us, 20_000);
    assert_eq!(report.depth_us, buffer.buffered() as u32 * FRAME_US);
    assert_eq!(report.output_us, 5_300);
    assert_eq!(report.received, 400);
    assert!((9_900..=10_000).contains(&report.jitter_us), "{report:?}");
    // Huge jitter stays at the largest target.
    let mut wild = JitterBuffer::default();
    for sequence in 0..200u32 {
        let host = u64::from(sequence) * FRAME;
        let delay = if sequence % 2 == 0 { 0 } else { 1_000_000 };
        wild.push(sequence, host, host + delay, sequence);
    }
    assert_eq!(wild.target(), MAX_TARGET);
}

#[test]
fn audio_task_jitter_trims_a_backlog_and_restarts_after_a_jump() {
    let mut buffer = JitterBuffer::default();
    // A burst of 30 frames at once (a stalled network catching up).
    steady(&mut buffer, 0..1, 0);
    assert_eq!(buffer.pop(), Playout::Frame(0));
    for sequence in 1..31u32 {
        buffer.push(sequence, u64::from(sequence) * FRAME, 0, sequence);
    }
    let mut played = 0;
    while let Playout::Frame(_) = buffer.pop() {
        played += 1;
    }
    // Every frame was played or trimmed, and the backlog was cut.
    let trimmed = buffer.stats().trimmed;
    assert!(trimmed >= 1, "{:?}", buffer.stats());
    assert_eq!(played + trimmed, 30);
    // More than MAX_FRAMES waiting: the oldest go first.
    let mut full = JitterBuffer::default();
    steady(&mut full, 0..(MAX_FRAMES as u32 + 10), 0);
    assert_eq!(full.buffered(), MAX_FRAMES);
    assert_eq!(full.pop(), Playout::Frame(10));
    // A new capture process starts its sequence anew: the buffer restarts.
    let mut restarted = JitterBuffer::default();
    steady(&mut restarted, 5_000..5_002, 0);
    assert_eq!(restarted.pop(), Playout::Frame(5_000));
    steady(&mut restarted, 0..2, 0);
    assert_eq!(restarted.stats().resets, 1);
    assert_eq!(restarted.pop(), Playout::Frame(0));
    // Switching the sound off forgets the frames, not the counters.
    restarted.clear();
    assert_eq!(restarted.buffered(), 0);
    assert_eq!(restarted.stats().resets, 1);
}

#[test]
fn audio_task_jitter_sequence_wraps_without_a_restart() {
    let mut buffer = JitterBuffer::default();
    let start = u32::MAX - 2;
    for step in 0..6u32 {
        let sequence = start.wrapping_add(step);
        buffer.push(
            sequence,
            u64::from(step) * FRAME,
            u64::from(step) * FRAME,
            step,
        );
    }
    let played: Vec<_> = (0..6).map(|_| buffer.pop()).collect();
    assert_eq!(played, (0..6).map(Playout::Frame).collect::<Vec<_>>());
    assert_eq!(buffer.stats().resets, 0);
}
