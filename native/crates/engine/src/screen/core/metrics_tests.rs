use super::*;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::synthetic::{render, static_block};

fn pong(ping_sent_us: u64, received_us: u64, sent_us: u64) -> Pong {
    Pong {
        sequence: 1,
        ping_sent_us,
        received_us,
        sent_us,
    }
}

#[test]
fn screen_task_metrics_latency_rates_and_clock() {
    // Nearest-rank percentiles over 1–100 ms.
    let mut latency = LatencySamples::default();
    assert_eq!(latency.summary(), None);
    for ms in 1..=100u64 {
        latency.record(ms * 1000);
    }
    let summary = latency.summary().unwrap();
    assert_eq!(
        (summary.p50_ms, summary.p95_ms, summary.max_ms),
        (50.0, 95.0, 100.0)
    );
    assert_eq!(latency.count(), 100);

    // 30 frames of 10 kB in the first second, 10 in the second.
    let mut rates = RateMeter::default();
    for frame in 0..30 {
        rates.record(frame * 33_333, 10_000);
    }
    for frame in 0..10 {
        rates.record(1_000_000 + frame * 100_000, 10_000);
    }
    let summary = rates.summary(2_000_000);
    assert_eq!(summary.frames, 40);
    assert_eq!((summary.fps_mean, summary.fps_min), (20.0, 10.0));
    assert_eq!((summary.kbps_mean, summary.kbps_max), (1600.0, 2400.0));
    // A completed second without frames counts as 0 frames/s.
    assert_eq!(rates.summary(3_500_000).fps_min, 0.0);

    // Host clock 5 s ahead, one-way delays 10 ms and 30 ms, turnaround 1 ms.
    let mut clock = ClockSync::default();
    let first = clock
        .on_pong(&pong(1_000_000, 6_010_000, 6_011_000), 1_041_000)
        .unwrap();
    assert_eq!((first.rtt_us, first.offset_us), (40_000, 4_990_000));
    // A shorter, symmetric round trip replaces it and gives the true offset.
    clock
        .on_pong(&pong(2_000_000, 7_005_000, 7_005_500), 2_010_500)
        .unwrap();
    assert_eq!(clock.offset_us(), Some(5_000_000));
    // A slower exchange soon after is kept out.
    clock.on_pong(&pong(3_000_000, 8_080_000, 8_080_000), 3_100_000);
    assert_eq!(clock.offset_us(), Some(5_000_000));
    // After 30 s the next exchange replaces it even when slower (drift).
    clock.on_pong(&pong(40_000_000, 45_040_000, 45_040_000), 40_060_000);
    assert_eq!(clock.offset_us(), Some(5_010_000));
    // Clocks that run backwards are ignored.
    assert_eq!(
        clock.on_pong(&pong(50_000_000, 55_000_000, 54_000_000), 50_100_000),
        None
    );
    assert_eq!(
        clock.on_pong(&pong(50_000_000, 55_000_000, 55_000_000), 40_000_000),
        None
    );
    // Capture on the host to display on the viewer.
    assert_eq!(clock.latency_us(45_100_000, 40_150_000), Some(60_000));
    assert_eq!(ClockSync::default().latency_us(1, 2), None);

    // The first disruption counts until the next good frame.
    let mut recovery = RecoveryTracker::default();
    recovery.on_disruption(1_000);
    recovery.on_disruption(2_000);
    assert_eq!(recovery.on_good_frame(251_000), Some(250_000));
    assert_eq!(recovery.on_good_frame(300_000), None);
    assert_eq!(recovery.recoveries_ms(), [250.0]);
}

#[test]
fn screen_task_metrics_static_psnr() {
    // The static text block of the synthetic source does not change between
    // frames: lossless content gives an infinite PSNR there.
    let size = FrameSize::new(640, 360);
    let (mut first, mut second) = (BgraImage::new(size).unwrap(), BgraImage::new(size).unwrap());
    render(10, &mut first).unwrap();
    render(11, &mut second).unwrap();
    let block = static_block(size);
    let psnr = psnr_db(&first.view(), &second.view(), block).unwrap();
    assert_eq!(psnr, f64::INFINITY);
    let whole = Rect {
        x: 0,
        y: 0,
        width: 640,
        height: 360,
    };
    let changed = psnr_db(&first.view(), &second.view(), whole).unwrap();
    assert!(changed.is_finite() && changed > 0.0);
    // One channel of one pixel off by 255: MSE = 255² / 3, PSNR = 10 log10 3.
    let black = BgraImage::new(FrameSize::new(1, 1)).unwrap();
    let mut blue = black.clone();
    blue.data_mut()[0] = 0xFF;
    let pixel = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let value = psnr_db(&black.view(), &blue.view(), pixel).unwrap();
    assert!((value - 10.0 * 3f64.log10()).abs() < 1e-9);
    let outside = Rect {
        x: 600,
        y: 0,
        width: 41,
        height: 1,
    };
    assert!(psnr_db(&first.view(), &second.view(), outside).is_err());
}
