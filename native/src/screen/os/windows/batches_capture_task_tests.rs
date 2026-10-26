//! Windows owner/transition fixtures for the one final remote task suite.
use super::*;
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_source_contract::{HostCodecPlan, HostContract};
use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::core::host_backend::SourceRefusal;
use crate::screen::core::host_backend::{ScreenHostBackend, ScreenSource};
use crate::screen::core::window_list::SourceId;
use crate::screen::os::host_backend::WindowsBackend;
use std::sync::Mutex;

fn session(name: &str) -> (Arc<MemoryPool>, Arc<SessionAdmission>) {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            name,
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    (pool, admission)
}
fn geometry(size: FrameSize, kind: CaptureKind) -> CaptureGeometry {
    CaptureGeometry {
        native: size,
        delivered: size,
        limits: FrameLimits::RESOURCE,
        kind,
    }
}
fn output(size: FrameSize) -> HostOutputPlan {
    HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap()
}
fn activate(
    admission: Arc<SessionAdmission>,
    key: SourceKey,
    geometry: CaptureGeometry,
) -> CaptureBudget {
    let output = output(geometry.displayed());
    let plan = geometry
        .plan(key.clone(), CaptureGeneration::new(1), &output)
        .unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    CaptureBudget::activate(admission, key, geometry, output, grant).unwrap()
}
#[test]
fn batches_capture_windows_native_slots_match_wgc_dxgi_rotation() {
    let size = FrameSize::new(3840, 2160);
    let bytes = bgra_bytes(size).unwrap();
    for (kind, surfaces, pixels) in [
        (CaptureKind::Wgc, 2, 2),
        (CaptureKind::Dxgi(BgraRotation::Identity), 1, 2),
        (CaptureKind::Dxgi(BgraRotation::Clockwise90), 1, 3),
    ] {
        let geometry = geometry(size, kind);
        let plan = geometry
            .plan(
                SourceKey::new("slots", "monitor:DISPLAY1").unwrap(),
                CaptureGeneration::new(1),
                &output(geometry.displayed()),
            )
            .unwrap();
        let row = plan.phase_table()[HostPhase::Capture as usize];
        assert_eq!(row[O::NativeSurface as usize], bytes * surfaces);
        assert_eq!(row[O::Readback as usize], bytes);
        assert_eq!(row[O::SourcePixels as usize], bytes * pixels);
        assert_eq!(
            row[O::NativeSurface as usize]
                + row[O::Readback as usize]
                + row[O::SourcePixels as usize],
            bytes * (surfaces + 1 + pixels)
        );
    }
}
#[test]
fn batches_capture_windows_band_has_fixed_tail_capacity_and_explicit_sums() {
    let geometry = CaptureGeometry {
        native: FrameSize::new(8192, 4097),
        delivered: FrameSize::new(1920, 960),
        limits: FrameLimits::RESOURCE,
        kind: CaptureKind::Wgc,
    };
    let plan = geometry
        .plan(
            SourceKey::new("band", "monitor:DISPLAY1").unwrap(),
            CaptureGeneration::new(1),
            &output(geometry.displayed()),
        )
        .unwrap();
    let row = plan.phase_table()[HostPhase::Capture as usize];
    let band = 8192 * 64 * 4;
    assert_eq!(geometry.staging(), FrameSize::new(8192, 64));
    assert_eq!(row[O::Readback as usize], band * 2);
    assert_eq!(
        row[O::FilterScratch as usize],
        1920 * std::mem::size_of::<[u64; 4]>() as u64
    );
}
#[test]
fn batches_capture_windows_cache_alias_and_last_result_retain_session() {
    let (pool, admission) = session("retained");
    let size = FrameSize::new(64, 64);
    let budget = activate(
        admission.clone(),
        SourceKey::new("retained", "monitor:DISPLAY1").unwrap(),
        geometry(size, CaptureKind::Wgc),
    );
    let grant = budget.grant().unwrap().clone();
    let first = CaptureImageBuilder::new(size, FrameLimits::RESOURCE, &grant, O::SourcePixels)
        .unwrap()
        .freeze();
    let cached = first.clone();
    assert!(first.shares_pixels(&cached));
    assert_eq!(grant.live_bytes().unwrap(), bgra_bytes(size).unwrap());
    drop(cached);
    drop(first);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    let result = CaptureImageBuilder::new(size, FrameLimits::RESOURCE, &grant, O::SourcePixels)
        .unwrap()
        .freeze();
    drop(budget);
    drop(grant);
    drop(admission);
    assert!(pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "replacement"
        )
        .is_err());
    drop(result);
    assert!(pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "replacement"
        )
        .is_ok());
}
#[test]
fn batches_capture_windows_exhausted_slots_do_not_call_allocator() {
    let (_, admission) = session("full");
    let size = FrameSize::new(64, 64);
    let budget = activate(
        admission,
        SourceKey::new("full", "window:42").unwrap(),
        geometry(size, CaptureKind::Wgc),
    );
    let grant = budget.grant().unwrap();
    let bytes = bgra_bytes(size).unwrap();
    let old = grant.claim(O::SourcePixels, bytes).unwrap();
    let new = grant.claim(O::SourcePixels, bytes).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let denied = grant.claim(O::SourcePixels, bytes).and_then(|slice| {
        slice.allocate(|| {
            events.lock().unwrap().push("allocate");
            Ok(())
        })
    });
    assert!(denied.is_err());
    assert!(events.lock().unwrap().is_empty());
    drop(old);
    drop(new);
    struct NativeEvent {
        events: Arc<Mutex<Vec<&'static str>>>,
        grant: CaptureGrant,
        bytes: u64,
    }
    impl Drop for NativeEvent {
        fn drop(&mut self) {
            assert!(self.grant.live_bytes().unwrap() >= self.bytes);
            self.events.lock().unwrap().push("final_drop");
        }
    }
    events.lock().unwrap().push("grant");
    let object = budget
        .claim_native()
        .unwrap()
        .allocate(|| {
            events.lock().unwrap().push("allocate");
            Ok(NativeEvent {
                events: events.clone(),
                grant: grant.clone(),
                bytes: bytes * 2,
            })
        })
        .unwrap();
    events.lock().unwrap().push("publish");
    drop(object);
    assert_eq!(grant.live_bytes().unwrap(), 0);
    events.lock().unwrap().push("release");
    assert_eq!(
        *events.lock().unwrap(),
        ["grant", "allocate", "publish", "final_drop", "release"]
    );
}
#[test]
fn batches_capture_windows_resize_rejects_before_native_allocation_with_old_owner() {
    let (_, admission) = session("resize");
    let size = FrameSize::new(3840, 2160);
    let mut budget = activate(
        admission.clone(),
        SourceKey::new("resize", "monitor:DISPLAY1").unwrap(),
        geometry(size, CaptureKind::Wgc),
    );
    // A one-byte storage fixture keeps the complete old plan alive without
    // allocating a large real native surface in the deterministic task case.
    let old = budget.grant().unwrap().claim(O::SourcePixels, 1).unwrap();
    budget.suspend();
    assert!(budget.replace(geometry(size, CaptureKind::Wgc)).is_err());
    assert!(budget.grant().is_err());
    let mut allocated = false;
    let refused = budget.claim_native().and_then(|slice| {
        slice.allocate(|| {
            allocated = true;
            Ok(())
        })
    });
    assert!(refused.is_err());
    assert!(!allocated);
    drop(old);
    budget.replace(geometry(size, CaptureKind::Wgc)).unwrap();
    assert!(budget.grant().is_ok());
    assert!(budget.generation() > 1);
}
#[test]
fn batches_capture_windows_rotation_preserves_pixels_and_source_generation() {
    let (_, admission) = session("rotation");
    let size = FrameSize::new(2, 3);
    let budget = activate(
        admission,
        SourceKey::new("rotation", "monitor:DISPLAY1").unwrap(),
        geometry(size, CaptureKind::Dxgi(BgraRotation::Clockwise90)),
    );
    let grant = budget.grant().unwrap();
    let mut builder =
        CaptureImageBuilder::new(size, FrameLimits::RESOURCE, grant, O::SourcePixels).unwrap();
    for (value, pixel) in builder.pixels_mut().chunks_exact_mut(4).enumerate() {
        pixel[0] = value as u8;
    }
    let raw = builder.freeze();
    let rotated = raw
        .rotate(
            BgraRotation::Clockwise90,
            FrameLimits::RESOURCE,
            grant,
            O::SourcePixels,
        )
        .unwrap();
    assert_eq!(rotated.size(), FrameSize::new(3, 2));
    assert_eq!(
        rotated
            .image()
            .data()
            .chunks_exact(4)
            .map(|p| p[0])
            .collect::<Vec<_>>(),
        [4, 2, 0, 5, 3, 1]
    );
    assert_eq!(rotated.source(), raw.source());
    assert_eq!(rotated.generation(), raw.generation());
    assert!(!rotated.shares_pixels(&raw));
    assert_eq!(grant.live_bytes().unwrap(), bgra_bytes(size).unwrap() * 2);
}
#[test]
fn batches_capture_windows_grantless_native_and_login_entries_refuse() {
    let backend = WindowsBackend::new();
    assert!(backend
        .open_source(&SourceId::Monitor("DISPLAY1".into()), None)
        .is_err());
    assert!(backend
        .open_family_member(crate::remote_window::NativeWindowId(42))
        .is_err());
    assert!(crate::screen::os::login_source::open_login_source().is_err());
}

/// Interactive fixtures use the same preparation and grant boundary as production.
pub(crate) fn open_admitted(
    backend: &WindowsBackend,
    source: &SourceId,
    wanted: Option<FrameSize>,
) -> Result<Box<dyn ScreenSource>, crate::screen::core::host_backend::SourceRefusal> {
    use crate::screen::core::capture_source_contract::{
        HostCodecPlan, HostContract, HostOutputPlan,
    };
    use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
    use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
    let refuse = |error: std::io::Error| {
        crate::screen::core::host_backend::SourceRefusal::new(&error.to_string())
    };
    let pool = std::sync::Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "windows-fixture",
        )
        .map_err(refuse)?;
    admission.bind_contract(None).map_err(refuse)?;
    let prepared = backend.prepare_session_source(
        "fixture",
        "windows-fixture",
        source,
        wanted,
        false,
        admission.clone(),
    )?;
    let info = prepared.description();
    let size = info.capture_limits.fit(info.native_size.ok_or_else(|| {
        crate::screen::core::host_backend::SourceRefusal::new("fixture capture metadata pending")
    })?);
    let output =
        HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).map_err(refuse)?;
    let plan = prepared
        .capture_plan(&output)
        .map_err(refuse)?
        .ok_or_else(|| {
            crate::screen::core::host_backend::SourceRefusal::new("fixture plan pending")
        })?;
    let grant = admission.authority().approve(plan).map_err(refuse)?;
    prepared.activate(grant)
}
