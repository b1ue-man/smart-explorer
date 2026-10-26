//! Source-level fixtures for the final shared remote task suite.
use super::*;
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::host_backend::prepared::test_admission;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Refusing(Arc<AtomicUsize>);
impl CaptureAuthority for Refusing {
    fn approve(&self, _plan: CapturePlan) -> io::Result<CaptureGrant> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("fixture refuses before parser growth"))
    }
}
fn grant(session: &str) -> CaptureGrant {
    let admission = test_admission(session);
    let source = SourceKey::new(session, "selected-window").unwrap();
    let mut plan = CapturePlan::new(source, CaptureGeneration::new(1));
    plan.add_bytes(CaptureOwner::SourcePixels, 64, 2, CapturePhases::ALL)
        .unwrap();
    plan.add_bytes(CaptureOwner::Readback, 96, 1, CapturePhases::ALL)
        .unwrap();
    admission.authority().approve(plan).unwrap()
}
fn reply(sequence: u16, pixels: u32, width: u16, height: u16) -> [u8; 32] {
    let mut header = [0u8; 32];
    header[0] = 1;
    header[2..4].copy_from_slice(&sequence.to_ne_bytes());
    header[4..8].copy_from_slice(&pixels.to_ne_bytes());
    header[12..14].copy_from_slice(&width.to_ne_bytes());
    header[14..16].copy_from_slice(&height.to_ne_bytes());
    header
}
#[test]
fn batches_task_capture_linux_x11_cursor_header_precedes_parent_approval() {
    let grant = grant("cursor-refused");
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Gate::default();
    gate.cursor(7, &grant, Arc::new(Refusing(calls.clone())))
        .unwrap();
    {
        let mut state = gate.0.lock().unwrap();
        state.header = reply(7, 512 * 512, 512, 512);
        state.filled = 31;
    }
    assert!(!gate.authorize().unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    gate.0.lock().unwrap().filled = 32;
    assert!(gate.authorize().is_err());
    let state = gate.0.lock().unwrap();
    assert!(!state.allowed);
    assert!(state.raw.is_none());
    assert!(state.parsed.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[test]
fn batches_task_capture_linux_x11_oversized_cursor_is_rejected_before_credit_request() {
    let grant = grant("cursor-too-large");
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Gate::default();
    gate.cursor(8, &grant, Arc::new(Refusing(calls.clone())))
        .unwrap();
    {
        let mut state = gate.0.lock().unwrap();
        state.header = reply(8, 65535u32 * 65535, 65535, 65535);
        state.filled = 32;
    }
    assert!(gate.authorize().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let state = gate.0.lock().unwrap();
    assert!(!state.allowed);
    assert!(state.raw.is_none());
    assert!(state.parsed.is_none());
}
#[test]
fn batches_task_capture_linux_x11_exact_reply_and_immutable_last_owner() {
    let grant = grant("x11-immutable");
    let gate = Gate::default();
    gate.image(9, 64, &grant).unwrap();
    assert_eq!(grant.live_bytes().unwrap(), 96);
    {
        let mut state = gate.0.lock().unwrap();
        state.header = reply(9, 16, 0, 0);
        state.filled = 32;
    }
    assert!(gate.authorize().unwrap());
    // A returned image and its source cache share one actual pixel store.
    let image = CaptureImageBuilder::new(
        FrameSize::new(4, 4),
        FrameLimits::RESOURCE,
        &grant,
        CaptureOwner::SourcePixels,
    )
    .unwrap()
    .freeze();
    let held = image.clone();
    assert!(image.shares_pixels(&held));
    gate.finish().unwrap();
    assert_eq!(grant.live_bytes().unwrap(), 64);
    drop(image);
    assert_eq!(grant.live_bytes().unwrap(), 64);
    drop(held);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_capture_linux_x11_malformed_reply_never_opens_parser_gate() {
    let grant = grant("x11-malformed");
    let gate = Gate::default();
    gate.image(11, 64, &grant).unwrap();
    {
        let mut state = gate.0.lock().unwrap();
        state.header = reply(11, 1_000_000, 0, 0);
        state.filled = 32;
    }
    assert!(gate.authorize().is_err());
    assert!(!gate.0.lock().unwrap().allowed);
    // The uncertain reply stays charged until the connection/gate ends.
    assert_eq!(grant.live_bytes().unwrap(), 96);
    drop(gate);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_capture_linux_x11_shm_and_getimage_count_distinct_native_owners() {
    let source = SourceKey::new("x11-plan", "selected-window").unwrap();
    let size = FrameSize::new(100, 50);
    let n = 20_000;
    let pixmap = 104 * 54 * 4; // The same window with its actual two-pixel border.
    let shm = super::super::capture_memory::plan(
        &source,
        CaptureGeneration::new(1),
        size,
        true,
        pixmap,
        None,
    )
    .unwrap();
    assert_eq!(
        shm.owner_limits()[CaptureOwner::SourcePixels as usize],
        2 * n
    );
    assert_eq!(
        shm.owner_limits()[CaptureOwner::NativeSurface as usize],
        n + pixmap
    );
    let fallback = super::super::capture_memory::plan(
        &source,
        CaptureGeneration::new(1),
        size,
        false,
        pixmap,
        None,
    )
    .unwrap();
    assert_eq!(
        fallback.owner_limits()[CaptureOwner::Readback as usize],
        n + 32
    );
    assert_eq!(
        fallback.owner_limits()[CaptureOwner::NativeSurface as usize],
        pixmap
    );
}
