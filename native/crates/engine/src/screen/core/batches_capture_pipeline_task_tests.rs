//! Productive preparation fixtures consumed by the single remote task suite.
use super::{
    capture_admission::{CaptureGeneration, CaptureGrant, CapturePlan, SourceKey},
    capture_source_contract::{HostOutputPlan, PreparedSource, PreparedSourceInfo},
    frame_budget::FrameLimits,
    host_backend::{ScreenHostBackend, ScreenSource, SourceFrame, SourceRefusal},
    host_codec_adapters::{HostCodecPlan, HostContract},
    pixel_convert::FrameSize,
    session_state::SessionState,
    window_list::SourceId,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct Pending {
    key: SourceKey,
    activated: Arc<AtomicBool>,
}
impl PreparedSource for Pending {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        PreparedSourceInfo {
            native_size: None,
            capture_limits: FrameLimits::RESOURCE,
            supports_regions: true,
            state: SessionState::WaitingForPortal,
            generation: CaptureGeneration::new(1),
        }
    }
    fn capture_plan(&self, _output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        Ok(None)
    }
    fn activate(
        self: Box<Self>,
        _grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        self.activated.store(true, Ordering::SeqCst);
        Err(SourceRefusal::new("pending fixture must not activate"))
    }
}

#[test]
fn batches_task_capture_pipeline_unknown_portal_stays_pending_without_activation() {
    let admission = super::capture_test_support::session(None);
    let activated = Arc::new(AtomicBool::new(false));
    let pending = Box::new(Pending {
        key: SourceKey::new(admission.session(), "portal").unwrap(),
        activated: activated.clone(),
    });
    let output = HostOutputPlan::new(
        HostContract::Legacy,
        FrameSize::new(320, 180),
        HostCodecPlan::Tiles,
    )
    .unwrap();
    let mut source =
        super::host_admission::pending(pending, admission.clone(), output, Arc::new(|| true));
    assert_eq!(source.size(), FrameSize::new(0, 0));
    assert!(matches!(
        source.capture(1, true).unwrap(),
        SourceFrame::Paused(SessionState::WaitingForPortal)
    ));
    assert!(source.capture_grant().is_none());
    assert_eq!(admission.additional_bytes().unwrap(), 0);
    assert!(!activated.load(Ordering::SeqCst));
}

#[test]
fn batches_task_capture_pipeline_synthetic_requires_plan_and_counts_aliases_and_busy_slots() {
    let backend = crate::screen::os::synthetic_backend::SyntheticBackend;
    assert!(backend.open_source(&SourceId::Synthetic, None).is_err());
    let admission = super::capture_test_support::session(None);
    let size = FrameSize::new(320, 180);
    let prepared = backend
        .prepare_session_source(
            "fixture",
            admission.session(),
            &SourceId::Synthetic,
            Some(size),
            false,
            admission.clone(),
        )
        .unwrap();
    let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
    let plan = prepared.capture_plan(&output).unwrap().unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let book = grant.book_identity();
    let mut source = prepared.activate(grant).unwrap();
    let SourceFrame::Frame(first) = source.capture(1, true).unwrap() else {
        panic!("first synthetic frame missing");
    };
    let SourceFrame::Frame(second) = source.capture(1, true).unwrap() else {
        panic!("cached synthetic frame missing");
    };
    assert!(first.shares_pixels(&first.clone()));
    assert!(!first.shares_pixels(&second));
    assert_eq!(first.grant().book_identity(), book);
    assert_eq!(source.capture_grant().unwrap().book_identity(), book);
    assert!(matches!(
        source.capture(2, true).unwrap(),
        SourceFrame::Unchanged
    ));
    drop(second);
    assert!(matches!(
        source.capture(3, true).unwrap(),
        SourceFrame::Frame(_)
    ));
}

#[test]
fn batches_task_capture_source_only_accepts_native_width_above_legacy_output_side() {
    let native = FrameSize::new(5120, 1440);
    assert!(HostOutputPlan::new(HostContract::Legacy, native, HostCodecPlan::Tiles).is_err());
    assert!(
        HostOutputPlan::for_source(HostContract::Legacy, native, FrameLimits::RESOURCE).is_ok()
    );
}

#[test]
fn batches_task_capture_prepared_drop_ends_new_import_authority_but_keeps_live_mapping() {
    let admission = super::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "portal").unwrap();
    let prepared = super::host_admission::registered(
        Box::new(Pending {
            key: key.clone(),
            activated: Arc::new(AtomicBool::new(false)),
        }),
        admission.clone(),
    )
    .unwrap();
    let connection = admission.import_connection(&key).unwrap();
    let need = super::capture_import::CaptureImportNeed::new(
        key.clone(),
        connection.id(1).unwrap(),
        4096,
        0,
    )
    .unwrap();
    let imported = admission.authority().approve_import(need).unwrap();
    let mapping = imported.claim_mapping().unwrap();
    drop(prepared);
    let next = super::capture_import::CaptureImportNeed::new(
        key.clone(),
        connection.id(2).unwrap(),
        4096,
        0,
    )
    .unwrap();
    assert!(admission.authority().approve_import(next).is_err());
    assert!(admission.import_connection(&key).is_err());
    assert_eq!(mapping.bytes(), 4096);
    drop(imported);
    assert_eq!(mapping.bytes(), 4096);
}

#[test]
fn batches_task_capture_old_prepared_owner_cannot_close_its_replacement() {
    let admission = super::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "portal").unwrap();
    let make = || {
        super::host_admission::registered(
            Box::new(Pending {
                key: key.clone(),
                activated: Arc::new(AtomicBool::new(false)),
            }),
            admission.clone(),
        )
        .unwrap()
    };
    let old = make();
    let old_connection = admission.import_connection(&key).unwrap();
    let replacement = make();
    let connection = admission.import_connection(&key).unwrap();
    drop(old);
    drop(old_connection);
    let need = super::capture_import::CaptureImportNeed::new(
        key.clone(),
        connection.id(1).unwrap(),
        4096,
        0,
    )
    .unwrap();
    assert!(admission.authority().approve_import(need).is_ok());
    drop(replacement);
    assert!(admission.import_connection(&key).is_err());
}

#[test]
fn batches_task_capture_failed_metadata_constructor_releases_its_early_owner() {
    let admission = super::capture_test_support::session(None);
    let key = SourceKey::new(admission.session(), "portal").unwrap();
    let result = super::host_admission::prepare_registered(key.clone(), admission.clone(), || {
        let _connection = admission.import_connection(&key).unwrap();
        Err(SourceRefusal::new("metadata constructor refused"))
    });
    assert!(result.is_err());
    assert!(admission.import_connection(&key).is_err());
}

#[test]
fn batches_task_capture_source_switch_reuses_one_bank_with_fresh_generation_identity() {
    use super::capture_admission::CaptureOwner;
    use super::capture_plan::CapturePhases;
    use super::frame_memory::{CapacityCeiling, MemoryPool};
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(8 * 1024 * 1024),
            1024 * 1024,
            "source-switch",
        )
        .unwrap();
    let caps =
        super::frame_budget::FrameCapabilities::admitted(FrameSize::new(320, 180), 1024 * 1024)
            .unwrap();
    let contract = super::frame_budget::FrameContract::negotiate(Some(caps), Some(caps)).unwrap();
    admission.bind_contract(contract).unwrap();
    let replacement = admission.new_source_session().unwrap();
    assert_ne!(admission.session(), replacement.session());
    assert_eq!(pool.committed_bytes(), 1024 * 1024);
    let make = |session: &str, generation| {
        let mut plan = CapturePlan::new(
            SourceKey::new(session, "monitor").unwrap(),
            CaptureGeneration::new(generation),
        );
        plan.bind_contract(contract).unwrap();
        plan.add_bytes(
            CaptureOwner::SourcePixels,
            2 * 1024 * 1024,
            1,
            CapturePhases::ALL,
        )
        .unwrap();
        plan
    };
    let old = admission
        .authority()
        .approve(make(admission.session(), 3))
        .unwrap();
    let new = replacement
        .authority()
        .approve(make(replacement.session(), 1))
        .unwrap();
    assert_eq!(admission.additional_bytes().unwrap(), 3 * 1024 * 1024);
    assert_eq!(replacement.additional_bytes().unwrap(), 3 * 1024 * 1024);
    assert!(replacement
        .authority()
        .approve(make(admission.session(), 4))
        .is_err());
    drop(old);
    drop(new);
    assert_eq!(pool.committed_bytes(), 1024 * 1024);
}
