//! Task tests of the fix of 2026-11-18 (host log of a Windows VM): a
//! duplication refused with `E_INVALIDARG` after a loss settles before it
//! falls back, and the monitor no longer switches between Desktop
//! Duplication and Graphics Capture every 2 s.
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{E_ACCESSDENIED, E_INVALIDARG};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_UNSUPPORTED;

use super::{next_plan, STABLE_AFTER};
use crate::screen::core::capture_fallback::{FallbackStep, RetryPlan, RETRY_FIRST, RETRY_LONGEST};
use crate::screen::os::capture_source::{
    rebuild_fallback, settling, FallbackCause, FALLBACK_AFTER, SETTLE_STEP,
};

#[test]
fn shadowhost_task_invalidarg_rebuild_settles_before_fallback() {
    let step = FallbackStep::DuplicateOutput;
    let early = Duration::from_millis(300);
    assert!(SETTLE_STEP < FALLBACK_AFTER);
    // E_INVALIDARG of DuplicateOutput settles for FALLBACK_AFTER.
    assert!(settling(step, Some(E_INVALIDARG), Duration::ZERO));
    assert!(settling(step, Some(E_INVALIDARG), early));
    assert!(!settling(step, Some(E_INVALIDARG), FALLBACK_AFTER));
    // Only that step and code.
    assert!(!settling(
        FallbackStep::AcquireNextFrame,
        Some(E_INVALIDARG),
        early
    ));
    assert!(!settling(step, Some(DXGI_ERROR_UNSUPPORTED), early));
    assert!(!settling(step, Some(E_ACCESSDENIED), early));
    assert!(!settling(step, None, early));
    // The fallback waits as long, then names the loss.
    let kind = std::io::ErrorKind::Other;
    assert_eq!(
        rebuild_fallback(step, kind, Some(E_INVALIDARG), true, early),
        None
    );
    assert_eq!(
        rebuild_fallback(step, kind, Some(E_INVALIDARG), true, FALLBACK_AFTER),
        Some(FallbackCause::AccessLost)
    );
    assert_eq!(
        rebuild_fallback(step, kind, Some(E_INVALIDARG), false, FALLBACK_AFTER),
        Some(FallbackCause::Failed)
    );
}

#[test]
fn shadowhost_task_monitor_fallback_backoff_survives_short_return() {
    let start = Instant::now();
    // The first fallback, or one long after the first path came back.
    assert_eq!(next_plan(None, start).wait(), RETRY_FIRST);
    let held = start + STABLE_AFTER;
    assert_eq!(
        next_plan(Some((start, RetryPlan::new(start))), held).wait(),
        RETRY_FIRST
    );
    // The logged cycle: back on Desktop Duplication, lost again within
    // seconds. Each round doubles the wait up to RETRY_LONGEST.
    let mut now = start;
    let mut plan = next_plan(None, now);
    let mut waits = vec![plan.wait()];
    for _ in 0..7 {
        now += plan.wait() + Duration::from_secs(3);
        plan = next_plan(Some((now, plan)), now + Duration::from_secs(3));
        waits.push(plan.wait());
    }
    let seconds: Vec<u64> = waits.iter().map(Duration::as_secs).collect();
    assert_eq!(seconds, [2, 4, 8, 16, 32, 60, 60, 60]);
    assert_eq!(plan.wait(), RETRY_LONGEST);
    // The continued plan is due only after its doubled wait.
    let due = next_plan(Some((now, plan)), now);
    assert!(!due.due(now + Duration::from_secs(59)));
    assert!(due.due(now + RETRY_LONGEST));
}
