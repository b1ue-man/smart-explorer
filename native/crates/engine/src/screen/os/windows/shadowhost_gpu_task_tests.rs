//! Task tests of the fix of 2026-11-18 (host log of a Windows VM with an
//! NVIDIA GPU) without a GPU: a change of the capture path never switches
//! the GPU path off for the session, a retired grant is turnover, and an
//! encoder whose GPU path is off keeps a device of its own.
use std::io;

use super::super::turnover as encode_turnover;
use super::{input_route, GpuSwitch, InputRoute, Switched, SystemReason};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_test_support;
use crate::screen::core::pixel_convert::FrameSize;

#[test]
fn shadowhost_task_gpu_switch_holds_for_its_capture_device_only() {
    let (duplication, graphics_capture) = (0x1000_usize, 0x2000_usize);
    let mut switch = GpuSwitch::new();
    assert!(!switch.is_off());
    // An encoder of the earlier capture device fails after the capture went
    // to another path: the current device keeps the GPU path.
    assert_eq!(
        switch.fail(Some(duplication), Some(graphics_capture), "earlier"),
        Switched::Earlier
    );
    assert!(!switch.is_off());
    // A failure on the current device switches it off, once.
    assert_eq!(
        switch.fail(
            Some(graphics_capture),
            Some(graphics_capture),
            "video processor"
        ),
        Switched::Off
    );
    assert!(switch.is_off());
    assert_eq!(
        switch.fail(Some(graphics_capture), Some(graphics_capture), "again"),
        Switched::AlreadyOff
    );
    // The same device stays off; a new capture device tries again and the
    // reason of the switch is returned for the host log.
    assert_eq!(switch.capture_on(graphics_capture), None);
    assert!(switch.is_off());
    assert_eq!(
        switch.capture_on(duplication).as_deref(),
        Some("video processor")
    );
    assert!(!switch.is_off());
    // Before any capture (a check): off until a capture device appears.
    let mut early = GpuSwitch::new();
    assert_eq!(early.fail(None, None, "check"), Switched::Off);
    assert_eq!(early.capture_on(duplication).as_deref(), Some("check"));
}

#[test]
fn shadowhost_task_retired_grant_is_turnover_not_gpu_failure() {
    let grant = capture_test_support::grant(FrameSize::new(4, 2), None);
    let failure = io::Error::other("NVIDIA H.264 Encoder MFT: device removed");
    assert!(!encode_turnover(&grant, &failure));
    // The claim on the grant after the capture changed its path.
    grant.retire().unwrap();
    let refused = grant.claim(CaptureOwner::SourcePixels, 8).unwrap_err();
    assert!(CaptureGrant::is_retired_claim(&refused));
    assert!(encode_turnover(&grant, &refused));
    // Any error under a retired grant is turnover.
    assert!(encode_turnover(&grant, &failure));
    // The texture input skips the picture instead of failing.
    let skipped = super::turnover(refused);
    assert_eq!(skipped.kind(), io::ErrorKind::WouldBlock);
    let kept = super::turnover(io::Error::other("map failed"));
    assert_eq!(kept.kind(), io::ErrorKind::Other);
}

#[test]
fn shadowhost_task_cpu_path_keeps_hardware_encoder_on_own_device() {
    // The GPU path off (the log's "system memory input (Disabled)") or a
    // capture device without video support: a device of its own for a
    // D3D11-aware encoder; system memory only without D3D11.
    for reason in [SystemReason::Disabled, SystemReason::NoVideo] {
        assert_eq!(input_route(Err(reason), true), InputRoute::OwnDevice);
        assert_eq!(input_route(Err(reason), false), InputRoute::SystemMemory);
    }
    assert_eq!(
        input_route(Err(SystemReason::ManagerRefused), true),
        InputRoute::SystemMemory
    );
}
