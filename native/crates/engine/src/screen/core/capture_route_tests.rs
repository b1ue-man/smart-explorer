//! Plan 2026-11-17 P9 (GPU encode route): the pointer patch drawn into a
//! picture on the GPU, the conversion cause the chain report shows, the
//! quality target of sharp mode and the bench's requested rate.
use super::pointer_patch;
use crate::screen::core::chain_meter::shown_fallback;
use crate::screen::core::frame_timing::CaptureFallback;
use crate::screen::core::pixel_convert::{FrameSize, Rect};
use crate::screen::core::rate_control::model::{
    session_quality, set_session_quality, SHARP_TARGET_QUALITY,
};

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn gpu_encode_task_pointer_patch_is_even_and_inside_the_picture() {
    let size = FrameSize::new(2880, 1800);
    // Inside: widened to even edges, so every 2 x 2 chroma pair is whole.
    assert_eq!(
        pointer_patch(size, 100, 50, 32, 32),
        Some(rect(100, 50, 32, 32))
    );
    assert_eq!(
        pointer_patch(size, 101, 51, 32, 32),
        Some(rect(100, 50, 34, 34))
    );
    assert_eq!(
        pointer_patch(size, 101, 51, 31, 31),
        Some(rect(100, 50, 32, 32))
    );
    // Clipped at the top left and the bottom right.
    assert_eq!(
        pointer_patch(size, -10, -5, 32, 32),
        Some(rect(0, 0, 22, 28))
    );
    assert_eq!(
        pointer_patch(size, 2870, 1790, 32, 32),
        Some(rect(2870, 1790, 10, 10))
    );
    // An odd picture: only its even part (the NV12 picture) is drawn into.
    let odd = FrameSize::new(1921, 1081);
    assert_eq!(
        pointer_patch(odd, 1915, 1075, 32, 32),
        Some(rect(1914, 1074, 6, 6))
    );
    // Outside or empty: nothing to draw.
    assert_eq!(pointer_patch(size, 2880, 0, 32, 32), None);
    assert_eq!(pointer_patch(size, -40, 0, 32, 32), None);
    assert_eq!(pointer_patch(size, 0, 0, 0, 32), None);
    assert_eq!(pointer_patch(FrameSize::new(1, 1), 0, 0, 32, 32), None);
    // Every patch fits the picture and has even edges.
    for (left, top) in [(-31, -31), (0, 0), (7, 9), (2879, 1799), (1440, 900)] {
        if let Some(patch) = pointer_patch(size, left, top, 48, 48) {
            assert!(patch.fits(size), "{patch:?}");
            assert_eq!(patch.x % 2 + patch.y % 2, 0, "{patch:?}");
            assert_eq!(patch.width % 2 + patch.height % 2, 0, "{patch:?}");
        }
    }
}

#[test]
fn gpu_encode_task_conversion_cause_shows_only_beside_cpu_pixels() {
    let pointer = Some(CaptureFallback::GpuPointerBlend);
    assert_eq!(shown_fallback(pointer, Some("cpu")), pointer);
    assert_eq!(shown_fallback(pointer, None), pointer);
    // A picture on the GPU leaves an older conversion cause behind.
    assert_eq!(shown_fallback(pointer, Some("gpu")), None);
    // A fallback of the capture path stays in either case.
    let dxgi = Some(CaptureFallback::DxgiFailed);
    assert_eq!(shown_fallback(dxgi, Some("gpu")), dxgi);
    assert_eq!(shown_fallback(None, Some("cpu")), None);
    let conversion = CaptureFallback::ALL
        .into_iter()
        .filter(|cause| cause.is_conversion())
        .map(CaptureFallback::key)
        .collect::<Vec<_>>();
    assert_eq!(
        conversion,
        [
            "gpu_not_armed",
            "gpu_other_device",
            "gpu_disabled",
            "gpu_no_video",
            "gpu_rotated",
            "gpu_scaled",
            "gpu_pointer_blend",
        ]
    );
}

#[test]
fn gpu_encode_task_sharp_quality_target_and_bench_rate() {
    assert_eq!(SHARP_TARGET_QUALITY, 18);
    set_session_quality(Some(SHARP_TARGET_QUALITY));
    assert_eq!(session_quality(), Some(SHARP_TARGET_QUALITY));
    set_session_quality(None);
    assert_eq!(session_quality(), None);
    // The bench asks for the highest rate; the host bounds it.
    assert_eq!(
        u32::from(crate::screen::core::bench::DEFAULT_FPS),
        crate::screen::core::pacing::MAX_FPS
    );
}
