use super::*;
use crate::screen::core::frame_timing::{CaptureFacts, CaptureFallback, CapturePath};

#[test]
fn p9_task_fallback_detail_crosses_the_process_boundary_beside_the_facts() {
    let facts = CaptureFacts {
        path: Some(CapturePath::WindowsGraphicsCapture),
        fallback: Some(CaptureFallback::DxgiFailed),
        rate_capped: Some(true),
        cursor_embedded: None,
        on_gpu: Some(false),
    };
    for step in FallbackStep::ALL {
        for code in [None, Some(0x887A_0004), Some(0xFFFF_FFFF)] {
            let detail = FallbackDetail { step, code };
            let bits = facts.pack() | detail.pack();
            // The facts read as before; the detail comes back whole.
            assert_eq!(CaptureFacts::unpack(bits), facts);
            assert_eq!(FallbackDetail::unpack(bits), Some(detail));
        }
    }
    assert_eq!(FallbackDetail::unpack(facts.pack()), None);
    // A step of a newer process reads as „none“.
    assert_eq!(FallbackDetail::unpack(0xff << 24), None);
}

#[test]
fn p9_task_fallback_detail_names_the_step_and_the_result_code() {
    let detail = FallbackDetail {
        step: FallbackStep::DuplicateOutput,
        code: Some(0x887A_0004),
    };
    assert_eq!(
        detail.text(),
        "DuplicateOutput 0x887A0004 DXGI_ERROR_UNSUPPORTED"
    );
    let unknown = FallbackDetail {
        step: FallbackStep::AcquireNextFrame,
        code: Some(0x1234_5678),
    };
    assert_eq!(unknown.text(), "AcquireNextFrame 0x12345678");
    let none = FallbackDetail {
        step: FallbackStep::CaptureMemory,
        code: None,
    };
    assert_eq!(none.text(), "capture_memory (ohne Ergebniscode)");
    assert_eq!(
        cause_text("dxgi_failed", Some(detail)),
        "dxgi_failed: DuplicateOutput 0x887A0004 DXGI_ERROR_UNSUPPORTED"
    );
    assert_eq!(cause_text("dxgi_failed", None), "dxgi_failed");
    assert_eq!(code_name(0x887A_0026), Some("DXGI_ERROR_ACCESS_LOST"));
}

#[test]
fn p9_task_fallback_retry_plan_doubles_its_wait_up_to_the_longest() {
    let start = Instant::now();
    let mut plan = RetryPlan::new(start);
    assert!(!plan.due(start));
    assert!(plan.due(start + RETRY_FIRST));
    let mut now = start + RETRY_FIRST;
    let mut waits = Vec::new();
    for _ in 0..8 {
        plan.failed(now);
        waits.push(plan.wait());
        assert!(!plan.due(now));
        now += plan.wait();
        assert!(plan.due(now));
    }
    assert_eq!(waits[0], RETRY_FIRST * 2);
    assert_eq!(waits[1], RETRY_FIRST * 4);
    assert!(waits.windows(2).all(|pair| pair[1] >= pair[0]));
    assert_eq!(*waits.last().unwrap(), RETRY_LONGEST);
}

#[test]
fn p9_task_fallback_of_windows_ends_back_on_desktop_duplication() {
    let duplication = Some(CapturePath::DesktopDuplication);
    let graphics = Some(CapturePath::WindowsGraphicsCapture);
    for cause in [
        CaptureFallback::DxgiUnsupported,
        CaptureFallback::DxgiAccessLost,
        CaptureFallback::DxgiAdapterChanged,
        CaptureFallback::DxgiFailed,
    ] {
        assert!(fallback_ended(cause, duplication));
        assert!(!fallback_ended(cause, graphics));
        assert!(!fallback_ended(cause, None));
    }
    // A Linux fallback is not a Windows one.
    assert!(!fallback_ended(
        CaptureFallback::DmabufImport,
        Some(CapturePath::PipewireShm)
    ));
}
