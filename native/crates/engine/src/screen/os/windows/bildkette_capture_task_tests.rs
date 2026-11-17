//! Task tests of plan 2026-11-14 bildkette, package BK2b (Windows capture),
//! pure decisions: capture order and fallback (B-12), frame interval and
//! schedule of the capture thread (B-10, B-11), dirty-rectangle verdicts and
//! the duplication's pointer (B-13). No native call; the thread and the
//! fallback switch are in `bildkette_capture_thread_task_tests.rs`.
use std::time::Instant;

use super::*;
use crate::screen::core::pixel_convert::BgraRotation;
use crate::screen::os::capture_dxgi::pointer::{
    blend, decode, PointerImage, SHAPE_COLOR, SHAPE_MASKED_COLOR, SHAPE_MONOCHROME,
};
use crate::screen::os::capture_dxgi::regions::{verdict, Verdict};
use crate::screen::os::capture_dxgi::wait::{Cadence, Schedule, Step, MAX_BLOCK, MAX_INTERVAL};
use windows::Win32::Foundation::E_INVALIDARG;

#[test]
fn bildkette_task_win_capture_rank_monitor_duplication_first() {
    use CapturePath::{DesktopDuplication as Dd, GraphicsCapture as Gc};
    assert_eq!(capture_rank(true, true), [Dd, Gc]);
    assert_eq!(capture_rank(true, false), [Dd]);
    assert_eq!(capture_rank(false, true), [Gc]);
    assert!(capture_rank(false, false).is_empty());
}

#[test]
fn bildkette_task_win_capture_fallback_causes() {
    use windows::Win32::Foundation::E_ACCESSDENIED;
    use windows::Win32::Graphics::Dxgi::{
        DXGI_ERROR_NOT_CURRENTLY_AVAILABLE, DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED,
    };
    let now = Duration::ZERO;
    // A documented passing result is retried before Graphics Capture takes over.
    assert_eq!(
        fallback_cause(Some(DXGI_ERROR_UNSUPPORTED), false, now),
        None
    );
    assert_eq!(
        fallback_cause(Some(DXGI_ERROR_UNSUPPORTED), false, FALLBACK_AFTER),
        Some(FallbackCause::Unsupported)
    );
    assert_eq!(
        fallback_cause(
            Some(DXGI_ERROR_NOT_CURRENTLY_AVAILABLE),
            false,
            FALLBACK_AFTER
        ),
        Some(FallbackCause::Unsupported)
    );
    // Lost and not rebuilt.
    assert_eq!(
        fallback_cause(Some(DXGI_ERROR_UNSUPPORTED), true, FALLBACK_AFTER),
        Some(FallbackCause::AccessLost)
    );
    assert_eq!(
        fallback_cause(Some(E_INVALIDARG), true, now),
        Some(FallbackCause::AccessLost)
    );
    assert_eq!(
        fallback_cause(Some(E_INVALIDARG), false, now),
        Some(FallbackCause::Failed)
    );
    // The output left its adapter: no result code.
    assert_eq!(
        fallback_cause(None, false, now),
        Some(FallbackCause::AdapterChanged)
    );
    // Secure desktop and inactive session pause, never fall back.
    for code in [E_ACCESSDENIED, DXGI_ERROR_SESSION_DISCONNECTED] {
        assert_eq!(fallback_cause(Some(code), true, FALLBACK_AFTER), None);
    }
    let error = io::Error::other(windows::core::Error::from_hresult(DXGI_ERROR_UNSUPPORTED));
    assert_eq!(result_code(&error), Some(DXGI_ERROR_UNSUPPORTED));
    assert_eq!(result_code(&io::Error::other("output changed")), None);
}

#[test]
fn bildkette_task_win_capture_report_limits() {
    let mut report = CaptureReport {
        path: Some(CapturePath::DesktopDuplication),
        ..CaptureReport::default()
    };
    // No yellow border and no rate cap under Desktop Duplication.
    assert!(report.limits().is_empty());
    report.rate_capped = true;
    assert!(report.limits().is_empty());
    report.path = Some(CapturePath::GraphicsCapture);
    assert_eq!(report.limits(), ["capture_border", "capture_rate_capped"]);
    report.rate_capped = false;
    assert_eq!(report.limits(), ["capture_border"]);
    for key in [
        CapturePath::DesktopDuplication.key(),
        CapturePath::GraphicsCapture.key(),
        FallbackCause::Unsupported.key(),
        FallbackCause::AccessLost.key(),
        FallbackCause::AdapterChanged.key(),
        FallbackCause::Failed.key(),
        super::super::host_backend::LIMIT_CAPTURE_RATE_CAPPED,
    ] {
        assert!(key.len() <= 64, "{key}");
    }
}

#[test]
fn bildkette_task_win_capture_cadence_follows_requests() {
    let start = Instant::now();
    let mut cadence = Cadence::default();
    assert_eq!(cadence.interval(), None);
    let ms = |n: u64| start + Duration::from_millis(n);
    for at in [0, 10, 20, 20, 30, 50, 60] {
        // A burst (20 twice) is ignored, one skipped tick (30 to 50) does
        // not move the median.
        cadence.record(ms(at));
    }
    assert_eq!(cadence.interval(), Some(Duration::from_millis(10)));
    let mut idle = Cadence::default();
    idle.record(ms(0));
    idle.record(ms(5_000));
    assert_eq!(idle.interval(), Some(MAX_INTERVAL));
}

#[test]
fn bildkette_task_win_capture_schedule_reads_before_due() {
    let start = Instant::now();
    let ms = |n: u64| start + Duration::from_millis(n);
    let mut schedule = Schedule::default();
    let mut cadence = Cadence::default();
    // Unknown cadence: every compositor picture.
    assert_eq!(
        schedule.step(ms(0), &cadence),
        Step::Acquire {
            wait: MAX_BLOCK,
            due: None
        }
    );
    cadence.record(ms(0));
    cadence.record(ms(10));
    schedule.record_cost(Duration::from_millis(2));
    assert_eq!(schedule.lead(), Duration::from_millis(2));
    // Next request at 20: sleep until 18, then wait at most until 20.
    assert_eq!(
        schedule.step(ms(12), &cadence),
        Step::Sleep(Duration::from_millis(6))
    );
    assert_eq!(
        schedule.step(ms(18), &cadence),
        Step::Acquire {
            wait: Duration::from_millis(2),
            due: Some(ms(20))
        }
    );
    // Served: the next read is for the request at 30.
    schedule.served(ms(20));
    assert_eq!(
        schedule.step(ms(19), &cadence),
        Step::Sleep(Duration::from_millis(9))
    );
    // Overslept the start (read at 28 for 30, woke at 33): the request at
    // 30 is still open and is served late, not skipped for the one at 40.
    assert_eq!(
        schedule.step(ms(33), &cadence),
        Step::Acquire {
            wait: Duration::from_millis(10),
            due: Some(ms(30))
        }
    );
    // Long after the last request: the request of the current interval.
    assert_eq!(
        schedule.step(ms(1_005), &cadence),
        Step::Acquire {
            wait: Duration::from_millis(10),
            due: Some(ms(1_000))
        }
    );
}

#[test]
fn bildkette_task_win_capture_dirty_rectangles_decide_unchanged() {
    // No metadata: nothing changed, without reading the picture.
    assert_eq!(verdict(true, 0, None), Verdict::Unchanged);
    assert_eq!(verdict(false, 0, None), Verdict::Unchanged);
    assert_eq!(verdict(true, 64, Some((0, 0))), Verdict::Unchanged);
    assert_eq!(verdict(true, 64, Some((1, 0))), Verdict::Changed);
    assert_eq!(verdict(true, 64, Some((0, 3))), Verdict::Changed);
    // Unreadable or unadmitted (rotated) rectangles: the picture is read.
    assert_eq!(verdict(true, 64, None), Verdict::Unknown);
    assert_eq!(verdict(false, 64, Some((0, 0))), Verdict::Unknown);
}

#[test]
fn bildkette_task_win_capture_pointer_shapes_decode() {
    // Color: straight alpha becomes premultiplied RGBA.
    let color = [10, 20, 200, 255, 0, 0, 0, 0, 100, 100, 100, 128, 0, 0, 0, 0];
    let image = decode(SHAPE_COLOR, 2, 2, 8, (1, 0), &color).unwrap();
    assert_eq!(
        (image.width, image.height, image.hot_x, image.hot_y),
        (2, 2, 1, 0)
    );
    assert_eq!(&image.rgba[0..4], &[200, 20, 10, 255]);
    assert_eq!(image.rgba[7], 0);
    assert_eq!(image.rgba[11], 128);
    assert_eq!(&image.rgba[8..11], &[50, 50, 50]);
    // Masked color: alpha byte 0 replaces, 0xFF with black is transparent,
    // 0xFF with white inverts: black, and its transparent neighbors white.
    let masked = [0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255];
    let image = decode(SHAPE_MASKED_COLOR, 2, 2, 8, (0, 0), &masked).unwrap();
    assert_eq!(&image.rgba[0..4], &[255, 0, 0, 255]);
    assert_eq!(&image.rgba[4..8], &[0, 0, 0, 0]);
    assert_eq!(&image.rgba[8..12], &[0, 0, 0, 255]);
    assert_eq!(&image.rgba[12..16], &[255, 255, 255, 255]);
    // Monochrome: AND rows above XOR rows, one bit per pixel, MSB first.
    // Pixel 0: AND 1 XOR 0 transparent; 1: AND 0 XOR 0 black;
    // 2: AND 0 XOR 1 white; 3: AND 1 XOR 1 inverting.
    let mono = [0b1001_0000, 0b0011_0000];
    let image = decode(SHAPE_MONOCHROME, 4, 2, 1, (0, 0), &mono).unwrap();
    assert_eq!(image.height, 1);
    assert_eq!(&image.rgba[4..8], &[0, 0, 0, 255]);
    assert_eq!(&image.rgba[8..12], &[255, 255, 255, 255]);
    assert_eq!(&image.rgba[12..16], &[0, 0, 0, 255]);
    // Unknown type, empty, oversized or short buffers are refused.
    assert_eq!(decode(3, 2, 2, 8, (0, 0), &color), None);
    assert_eq!(decode(SHAPE_COLOR, 0, 2, 8, (0, 0), &color), None);
    assert_eq!(decode(SHAPE_COLOR, 513, 1, 2052, (0, 0), &[0; 2052]), None);
    assert_eq!(decode(SHAPE_COLOR, 2, 2, 8, (0, 0), &color[..12]), None);
}

#[test]
fn bildkette_task_win_capture_pointer_blends_into_picture() {
    let red = PointerImage {
        width: 2,
        height: 1,
        hot_x: 0,
        hot_y: 0,
        rgba: vec![255, 0, 0, 255, 0, 0, 0, 0],
    };
    let size = FrameSize::new(4, 3);
    let pixel = |pixels: &[u8], x: usize, y: usize| {
        let at = (y * 4 + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    // Unrotated: the opaque pixel lands at (1, 2) as BGRA, the transparent
    // one leaves the picture as it was.
    let mut pixels = vec![0u8; 4 * 3 * 4];
    blend(&mut pixels, size, size, BgraRotation::Identity, &red, 1, 2);
    assert_eq!(pixel(&pixels, 1, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, 2, 2), [0, 0, 0, 0]);
    // Rotated 90 degrees: the displayed picture is 3 x 4; its pixel (0, 0)
    // is the texture's pixel (0, 2) (`rotate_bgra`).
    let mut pixels = vec![0u8; 4 * 3 * 4];
    blend(
        &mut pixels,
        size,
        size,
        BgraRotation::Clockwise90,
        &red,
        0,
        0,
    );
    assert_eq!(pixel(&pixels, 0, 2), [0, 0, 255, 255]);
    assert_eq!(pixels.iter().filter(|value| **value == 255).count(), 2);
    // A picture scaled to half samples the pointer at each target center.
    let small = FrameSize::new(2, 2);
    let large = FrameSize::new(4, 4);
    let block = PointerImage {
        width: 2,
        height: 2,
        hot_x: 0,
        hot_y: 0,
        rgba: [0, 255, 0, 255].repeat(4),
    };
    let mut pixels = vec![0u8; 2 * 2 * 4];
    blend(
        &mut pixels,
        small,
        large,
        BgraRotation::Identity,
        &block,
        0,
        0,
    );
    assert_eq!(&pixels[0..4], &[0, 255, 0, 255]);
    assert_eq!(&pixels[4..8], &[0, 0, 0, 0]);
    // A buffer of the wrong length is left alone.
    let mut short = vec![7u8; 5];
    blend(&mut short, size, size, BgraRotation::Identity, &red, 0, 0);
    assert_eq!(short, [7; 5]);
}
