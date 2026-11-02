//! C10 fixtures for the single remote batch suite; no privileged desktop needed.
use super::{
    capture_wire::{CaptureCall, Plan},
    wire::{self, Reply},
};
use crate::{
    login::os::shared::{
        capture_admission::ParentCapture, capture_lifetime::CaptureLifetime, child_reader,
        helper_capture,
    },
    screen::core::{
        capture_admission::{
            CaptureGeneration, CaptureGrant, CaptureOwner, SessionAdmission, SourceKey,
        },
        capture_plan::{CapturePhases, CapturePlan},
        capture_source_contract::{HostOutputPlan, PreparedSource, PreparedSourceInfo},
        frame_budget::{FrameLimits, MAX_WORKING_BYTES},
        frame_memory::{CapacityCeiling, MemoryPool},
        host_backend::{ScreenSource, SourceFrame, SourceRefusal},
        pixel_convert::FrameSize,
        session_state::SessionState,
    },
};
use std::{
    io,
    sync::{Arc, Mutex},
};

fn admission() -> (Arc<MemoryPool>, Arc<SessionAdmission>) {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool
        .admit_session(
            None,
            CapacityCeiling::bounded(MAX_WORKING_BYTES),
            MAX_WORKING_BYTES,
            "cookie",
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    (pool, admission)
}
fn plan(source: &str, native: u64) -> CapturePlan {
    let mut plan = CapturePlan::new(
        SourceKey::new("cookie", source).unwrap(),
        CaptureGeneration::new(1),
    );
    plan.bind_contract(None).unwrap();
    plan.add_bytes(CaptureOwner::NativeSurface, native, 1, CapturePhases::ALL)
        .unwrap();
    plan.add_bytes(CaptureOwner::SourcePixels, 4, 2, CapturePhases::ALL)
        .unwrap();
    plan
}
fn reply(id: u64) -> Reply {
    let mut reply = Reply::state("pixels");
    reply.width = 1;
    reply.height = 1;
    reply.pixel_bytes = 4;
    reply.capture_id = Some(id);
    reply
}
#[test]
fn batches_task_login_capture_disjoint_parent_slot_and_shared_capacity() {
    let (_pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    let descriptor = Plan::from_plan(&plan("login:first", 300 * 1024 * 1024)).unwrap();
    let child = descriptor.capture_plan("cookie", false).unwrap();
    let combined = descriptor.capture_plan("cookie", true).unwrap();
    assert_eq!(child.owner_limits()[CaptureOwner::Transport as usize], 0);
    assert_eq!(combined.peak_bytes(), child.peak_bytes() + 4);
    assert!(parent.approve(1, descriptor).unwrap().approved);
    assert!(parent
        .approve(
            2,
            Plan::from_plan(&plan("login:second", 300 * 1024 * 1024)).unwrap()
        )
        .is_err());
    let pixels = parent.pixels(&reply(1), 4).unwrap();
    assert!(
        parent.pixels(&reply(1), 4).is_err(),
        "one live raw reply consumes the disjoint parent slot"
    );
    drop(pixels);
    assert!(parent.pixels(&reply(1), 4).is_ok());
}
#[test]
fn batches_task_login_capture_cursor_reply_requires_live_main_without_raw_slot() {
    let (_pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    let mut cursor = CapturePlan::new(
        SourceKey::new("cookie", "login:source").unwrap(),
        CaptureGeneration::new(1),
    );
    cursor
        .add_bytes(CaptureOwner::Readback, 64, 1, CapturePhases::ALL)
        .unwrap();
    cursor
        .add_bytes(CaptureOwner::Metadata, 4, 1, CapturePhases::ALL)
        .unwrap();
    let descriptor = Plan::from_plan(&cursor).unwrap();
    assert_eq!(descriptor.raw_bytes, 0);
    let combined = descriptor.capture_plan("cookie", true).unwrap();
    assert_eq!(combined.owner_limits()[CaptureOwner::Transport as usize], 0);
    assert_eq!(combined.peak_bytes(), 68);
    assert!(parent.approve(1, descriptor.clone()).is_err());
    parent
        .approve(1, Plan::from_plan(&plan("login:source", 4)).unwrap())
        .unwrap();
    parent.approve(2, descriptor.clone()).unwrap();
    parent.approve(3, descriptor.clone()).unwrap();
    assert!(
        parent.pixels(&reply(2), 4).is_err(),
        "reply metadata cannot claim a BGRA parent slot"
    );
    parent.retire(1).unwrap();
    assert!(parent.approve(4, descriptor).is_err());
}
#[test]
fn batches_task_login_capture_header_and_retired_grant_refuse_allocation() {
    let (_pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    let descriptor = Plan::from_plan(&plan("login:source", 4)).unwrap();
    parent.approve(1, descriptor.clone()).unwrap();
    assert!(parent.approve(1, descriptor).is_err());
    assert!(parent.pixels(&reply(1), 8).is_err());
    assert!(parent.pixels(&reply(2), 4).is_err());
    parent.retire(1).unwrap();
    assert!(parent.pixels(&reply(1), 4).is_err());
    let mut header = std::io::Cursor::new([wire::PIXELS, 0, 0, 0, 4]);
    assert_eq!(wire::read_header(&mut header).unwrap(), (wire::PIXELS, 4));
    assert_eq!(
        header.position(),
        5,
        "no body is read to validate its header"
    );
}
#[test]
fn batches_task_login_capture_reader_rejects_wrong_cookie_and_pixel_probe() {
    let (_pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    let descriptor = Plan::from_plan(&plan("login:source", 4)).unwrap();
    let call = CaptureCall::Prepare {
        cookie: "wrong".into(),
        id: 1,
        plan: descriptor.clone(),
    };
    let mut bytes = Vec::new();
    wire::write_frame(
        &mut bytes,
        wire::CAPTURE,
        &serde_json::to_vec(&call).unwrap(),
    )
    .unwrap();
    let lifetime = CaptureLifetime::new(Some(parent.clone()));
    let input: child_reader::Input = Arc::new(Mutex::new(Box::new(io::sink())));
    assert!(child_reader::read_fixture(
        Box::new(io::Cursor::new(bytes)),
        input.clone(),
        "cookie",
        lifetime.clone()
    )
    .is_err());
    assert!(
        parent.approve(1, descriptor).is_ok(),
        "wrong cookie never installed its grant"
    );
    lifetime.confirmed_exit();

    let probe = CaptureLifetime::new(None);
    let mut bytes = Vec::new();
    wire::send(&mut bytes, &reply(1)).unwrap();
    wire::write_frame(&mut bytes, wire::PIXELS, &[0; 4]).unwrap();
    assert!(child_reader::read_fixture(
        Box::new(io::Cursor::new(bytes)),
        input,
        "cookie",
        probe.clone()
    )
    .is_err());
    probe.confirmed_exit();
}
#[test]
fn batches_task_login_capture_eof_exit_and_last_pixel_are_distinct_ends() {
    let (pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    parent
        .approve(1, Plan::from_plan(&plan("login:source", 4)).unwrap())
        .unwrap();
    let pixels = parent.pixels(&reply(1), 4).unwrap();
    let weak = Arc::downgrade(&parent);
    let lifetime = CaptureLifetime::new(Some(parent.clone()));
    let reader = lifetime.reader();
    drop(parent);
    drop(reader); // EOF alone cannot release the process's outstanding owners.
    assert!(weak.upgrade().is_some());
    lifetime.confirmed_exit();
    assert!(weak.upgrade().is_none());
    assert!(
        pool.reserve(None, MAX_WORKING_BYTES).is_err(),
        "returned pixels still own their grant"
    );
    drop(pixels);
    assert_eq!(
        pool.reserve(None, MAX_WORKING_BYTES).unwrap().bytes(),
        MAX_WORKING_BYTES
    );
}
#[test]
fn batches_task_login_capture_exit_still_waits_for_reader_and_close_refuses_grants() {
    let (_pool, admission) = admission();
    let parent = ParentCapture::from_test_admission(admission);
    let weak = Arc::downgrade(&parent);
    let lifetime = CaptureLifetime::new(Some(parent.clone()));
    let reader = lifetime.reader();
    parent.close();
    assert!(parent
        .approve(1, Plan::from_plan(&plan("login:source", 4)).unwrap())
        .is_err());
    drop(parent);
    lifetime.confirmed_exit();
    assert!(weak.upgrade().is_some());
    drop(reader);
    assert!(weak.upgrade().is_none());
}

struct Prepared {
    key: SourceKey,
    native: u64,
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl PreparedSource for Prepared {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        PreparedSourceInfo {
            native_size: Some(FrameSize::new(1, 1)),
            capture_limits: FrameLimits::LEGACY,
            supports_regions: false,
            state: SessionState::Starting,
            generation: CaptureGeneration::new(1),
        }
    }
    fn capture_plan(&self, _output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        self.events.lock().unwrap().push("prepared");
        Ok(Some(plan(self.key.source(), self.native)))
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        grant
            .check_identity(&self.key, CaptureGeneration::new(1))
            .map_err(|e| SourceRefusal::new(&e.to_string()))?;
        self.events.lock().unwrap().push("activated");
        Ok(Box::new(Source(grant)))
    }
}
struct Source(CaptureGrant);
impl ScreenSource for Source {
    fn size(&self) -> FrameSize {
        FrameSize::new(1, 1)
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.0.clone())
    }
    fn capture(&mut self, _frame: u64, _key: bool) -> io::Result<SourceFrame> {
        Ok(SourceFrame::Unchanged)
    }
}
#[test]
fn batches_task_login_capture_prepared_source_cannot_activate_after_refused_grant() {
    let (_pool, admission) = admission();
    let events = Arc::new(Mutex::new(Vec::new()));
    let prepared = |native| {
        Box::new(Prepared {
            key: SourceKey::new("cookie", "login:source").unwrap(),
            native,
            events: events.clone(),
        }) as Box<dyn PreparedSource>
    };
    assert!(
        helper_capture::activate_prepared(prepared(MAX_WORKING_BYTES), admission.clone()).is_err()
    );
    assert_eq!(*events.lock().unwrap(), vec!["prepared"]);
    let source = helper_capture::activate_prepared(prepared(4), admission).unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        vec!["prepared", "prepared", "activated"]
    );
    assert!(source.capture_grant().is_some());
}
