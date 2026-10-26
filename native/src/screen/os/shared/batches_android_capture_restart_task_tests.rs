//! Android restart, retained owners and epoch cases behind unchanged test entries.
use super::*;

fn capture_restart() -> (
    crate::screen::os::android_capture_admission::CaptureSession,
    Arc<crate::screen::core::capture_admission::SessionAdmission>,
    crate::screen::core::capture_admission::CaptureGrant,
    u64,
) {
    use crate::screen::{
        core::{
            capture_admission::{CaptureGeneration, SourceKey},
            host_codec_adapters::{HostCodecPlan, HostContract, HostOutputPlan},
        },
        os::android_capture_admission as android,
    };
    let size = FrameSize::new(64, 32);
    let working = 8 * MIB;
    let caps = FrameCapabilities::admitted(size, working).unwrap();
    let contract = frame_budget::FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap();
    let output = HostOutputPlan::new(
        HostContract::Versioned(contract),
        size,
        HostCodecPlan::OpenH264,
    )
    .unwrap();
    let source = SourceKey::new("android-restart", "synthetic").unwrap();
    let first = android::plan(
        &source,
        CaptureGeneration::new(1),
        size,
        android::SYNTHETIC_SURFACE,
        &output,
    )
    .unwrap();
    let peak = first.peak_bytes();
    let capacity = peak + MIB;
    assert!(2 * peak > capacity);
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(capacity),
            working,
            "android-restart",
        )
        .unwrap();
    admission.bind_contract(Some(contract)).unwrap();
    let capture = android::CaptureSession::default();
    capture.attach(admission.clone(), source).unwrap();
    let first = capture
        .plan(size, android::SYNTHETIC_SURFACE, &output)
        .unwrap();
    let grant = admission.authority().approve(first).unwrap();
    capture.activate(grant.clone()).unwrap();
    assert_eq!(capture.begin(size, android::SYNTHETIC_SURFACE).unwrap(), 1);
    (capture, admission, grant, peak)
}

pub(super) fn batches_task_b6_session_budget_android_resume_reuses_capacity_after_actual_owner_end() {
    use crate::screen::{
        core::capture_admission::CaptureOwner, os::android_capture_admission as android,
    };
    let (capture, admission, old, peak) = capture_restart();
    let encoder = old
        .claim(CaptureOwner::CodecOutput, android::ACCESS_UNIT)
        .unwrap();
    let surface = old.claim(CaptureOwner::NativeSurface, 64 * 32 * 4).unwrap();
    // The existing Java pause path acknowledges only after these real owners end.
    drop(encoder);
    drop(surface);
    assert_eq!(old.live_bytes().unwrap(), 0);
    let next = capture
        .begin(FrameSize::new(64, 32), android::SYNTHETIC_SURFACE)
        .unwrap();
    assert_eq!(next, 2);
    assert!(old.is_retired().unwrap());
    assert!(capture.grant(1).is_err());
    assert_eq!(capture.grant(next).unwrap().plan().peak_bytes(), peak);
    assert_eq!(
        admission.additional_bytes().unwrap(),
        peak - admission.working_bytes()
    );
}

pub(super) fn batches_task_b6_session_budget_android_resume_keeps_live_owner_and_retries_same_generation() {
    use crate::screen::{
        core::capture_admission::CaptureOwner, os::android_capture_admission as android,
    };
    let (capture, admission, old, peak) = capture_restart();
    let retiring = old
        .claim(CaptureOwner::CodecOutput, android::ACCESS_UNIT)
        .unwrap();
    let error = capture
        .begin(FrameSize::new(64, 32), android::SYNTHETIC_SURFACE)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert!(old.is_retired().unwrap());
    assert_eq!(old.live_bytes().unwrap(), android::ACCESS_UNIT);
    assert!(old.claim(CaptureOwner::CodecOutput, 1).is_err());
    let acknowledgement = capture.activity_ack(&[0; 9]).unwrap();
    assert_eq!(acknowledgement.len(), 9);
    assert_eq!(old.live_bytes().unwrap(), android::ACCESS_UNIT);
    assert_eq!(
        admission.additional_bytes().unwrap(),
        android::ACCESS_UNIT + 9 - admission.working_bytes()
    );
    assert!(capture.activity_ack(&[0; 10]).is_err());
    drop(acknowledgement);
    assert_eq!(
        admission.additional_bytes().unwrap(),
        android::ACCESS_UNIT - admission.working_bytes()
    );
    drop(retiring);
    assert_eq!(
        capture
            .begin(FrameSize::new(64, 32), android::SYNTHETIC_SURFACE)
            .unwrap(),
        2
    );
    assert_eq!(
        admission.additional_bytes().unwrap(),
        peak - admission.working_bytes()
    );
}

pub(super) fn batches_task_b6_session_budget_android_snapshot_replaces_only_retired_or_native_authority() {
    use crate::screen::{
        core::{
            capture_admission::{CaptureGeneration, CaptureOwner, SourceKey},
            host_codec_adapters::{HostCodecPlan, HostContract, HostOutputPlan},
        },
        os::android_capture_admission as android,
    };
    let (active, _, active_surface, _) = capture_restart();
    let parallel = active.synthetic_snapshot(FrameSize::new(64, 32)).unwrap();
    assert!(!active_surface.is_retired().unwrap());
    assert_eq!(
        active.current().unwrap().book_identity(),
        active_surface.book_identity()
    );
    assert_ne!(parallel.book_identity(), active_surface.book_identity());
    drop(parallel);
    drop(active);
    let (surface, _, old, _) = capture_restart();
    let live = old
        .claim(CaptureOwner::CodecOutput, android::ACCESS_UNIT)
        .unwrap();
    assert_eq!(
        surface
            .begin(FrameSize::new(64, 32), android::SYNTHETIC_SURFACE)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    let snapshot = surface.synthetic_snapshot(FrameSize::new(64, 32)).unwrap();
    assert!(!snapshot.is_retired().unwrap());
    assert_eq!(snapshot.generation().value(), 2);
    let pixels = snapshot
        .claim(CaptureOwner::SourcePixels, 64 * 32 * 4)
        .unwrap();
    assert_eq!(old.live_bytes().unwrap(), android::ACCESS_UNIT);
    drop(pixels);
    drop(live);
    assert_eq!(
        surface.current().unwrap().generation(),
        snapshot.generation()
    );

    let output_size = FrameSize::new(64, 32);
    let working = MIB;
    let caps = FrameCapabilities::admitted(output_size, working).unwrap();
    let contract = frame_budget::FrameContract::negotiate(Some(caps), Some(caps))
        .unwrap()
        .unwrap();
    let output = HostOutputPlan::new(
        HostContract::Versioned(contract),
        output_size,
        HostCodecPlan::Tiles,
    )
    .unwrap();
    let source = SourceKey::new("native-snapshot", "synthetic").unwrap();
    let first_size = FrameSize::new(1024, 512);
    let next_size = FrameSize::new(1024, 768);
    let first = android::plan(
        &source,
        CaptureGeneration::new(1),
        first_size,
        android::SYNTHETIC_PIXELS,
        &output,
    )
    .unwrap();
    let next = android::plan(
        &source,
        CaptureGeneration::new(2),
        next_size,
        android::SYNTHETIC_PIXELS,
        &output,
    )
    .unwrap();
    let capacity = next.peak_bytes() + 4096;
    assert!(first.peak_bytes() + next.peak_bytes() > capacity);
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(capacity),
            working,
            "native-snapshot",
        )
        .unwrap();
    admission.bind_contract(Some(contract)).unwrap();
    let native = android::CaptureSession::default();
    native.attach(admission.clone(), source).unwrap();
    let first = native
        .plan(first_size, android::SYNTHETIC_PIXELS, &output)
        .unwrap();
    let old = admission.authority().approve(first).unwrap();
    native.activate(old.clone()).unwrap();
    let held = old.claim(CaptureOwner::SourcePixels, 16).unwrap();
    let next = native.synthetic_snapshot(next_size).unwrap();
    assert_eq!(next.generation().value(), 2);
    assert!(old.is_retired().unwrap());
    assert_eq!(old.live_bytes().unwrap(), 16);
    assert_eq!(
        admission.additional_bytes().unwrap(),
        next.plan().peak_bytes() + 16 - working
    );
    let pixels = next
        .claim(CaptureOwner::SourcePixels, next_size.pixels() * 4)
        .unwrap();
    assert_eq!(
        native.synthetic_snapshot(next_size).unwrap().generation(),
        next.generation()
    );
    drop(pixels);
    drop(held);
    assert_eq!(old.live_bytes().unwrap(), 0);
}

pub(super) fn batches_task_b6_session_budget_android_epoch_drops_old_queue_but_keeps_external_owner() {
    use crate::screen::{
        core::{
            capture_admission::{CaptureOwner, SourceKey},
            host_codec_adapters::{HostCodecPlan, HostContract, HostOutputPlan},
        },
        os::{android_capture_admission as android, bridge_channels as channels},
    };
    let _lock = channels::TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    channels::reset_for_tests();
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(512 * MIB),
            512 * MIB,
            "epoch",
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    let channel = channels::open(channels::HostStart {
        source: "synthetic".into(),
        still: false,
        viewer_device_id: String::new(),
        viewer_name: String::new(),
        max_width: 64,
        max_height: 32,
    });
    channel
        .capture
        .attach(
            admission.clone(),
            SourceKey::new("epoch", "synthetic").unwrap(),
        )
        .unwrap();
    let size = FrameSize::new(64, 32);
    let n = size.pixels() * 4;
    let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
    let grant = admission
        .authority()
        .approve(
            channel
                .capture
                .plan(size, android::PROJECTION_TILES, &output)
                .unwrap(),
        )
        .unwrap();
    channel.capture.activate(grant.clone()).unwrap();
    let held = grant.claim(CaptureOwner::SourcePixels, n).unwrap();
    let mut picture = Vec::new();
    picture.extend_from_slice(&64u32.to_le_bytes());
    picture.extend_from_slice(&32u32.to_le_bytes());
    picture.resize(n as usize + 8, 0);
    assert_eq!(
        channels::push_admitted(
            channel.id,
            1,
            &picture,
            channels::FLAG_STILL | (1 << channels::EPOCH_SHIFT)
        ),
        channels::PUSH_OK
    );
    assert_eq!(grant.live_bytes().unwrap(), 2 * n + 8);
    channel.send(serde_json::json!({"op":"configure","epoch":2}));
    assert_eq!(grant.live_bytes().unwrap(), n);
    assert_eq!(
        channels::push_admitted(
            channel.id,
            1,
            &picture,
            channels::FLAG_STILL | (1 << channels::EPOCH_SHIFT)
        ),
        channels::PUSH_OK
    );
    assert_eq!(
        grant.live_bytes().unwrap(),
        n,
        "old-epoch payload acquired no new queue owner"
    );
    channels::close(channel.id, "viewer_closed");
    assert_eq!(grant.live_bytes().unwrap(), n);
    drop(held);
    assert_eq!(grant.live_bytes().unwrap(), 0);
}
