//! Controlled capture inputs exercise the production login/restart transitions.
//! Only the platform sampling/binding observations are replaced in these cases.
use super::*;
use std::collections::VecDeque;
use crate::screen::core::capture_admission::{CaptureOwner, CapturePlan, CaptureSlice};
use crate::screen::core::capture_image::CaptureImageBuilder;
use crate::screen::core::capture_plan::{bgra_bytes, CapturePhases};
use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryPool};
use crate::screen::core::host_codec_adapters::AdmittedTilesEncoder;
use crate::screen::core::capture_source_contract::{HostCodecPlan, HostContract};
use crate::screen::core::pixel_convert::BgraRotation;
use crate::screen::os::capture_prepared::recovery::CaptureRecovery;
use crate::screen::os::capture_slots::CaptureKind;

enum Sample { Image(CapturedImage), Idle, Paused, Rebind, Fail }
struct FakeCapture {
    binding: OutputBinding,
    samples: VecDeque<Sample>,
    last: Option<CapturedImage>,
    calls: Vec<bool>,
}
impl Grabber for FakeCapture {
    fn size(&self) -> FrameSize { self.binding.size }
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        self.calls.push(keyframe);
        match self.samples.pop_front().unwrap_or(Sample::Idle) {
            Sample::Image(image) => { self.last = Some(image.clone()); Ok(SourceFrame::Frame(image)) }
            Sample::Paused => Ok(SourceFrame::Paused(SessionState::Starting)),
            Sample::Rebind => { self.binding.generation += 1; Ok(SourceFrame::Unchanged) }
            Sample::Fail => Err(io::Error::new(io::ErrorKind::PermissionDenied, "capture context ended")),
            Sample::Idle => Ok(match (&self.last, keyframe) {
                (Some(image), true) => SourceFrame::Frame(image.clone()),
                _ => SourceFrame::Unchanged,
            }),
        }
    }
    fn reopen(&mut self) -> io::Result<()> { Err(io::Error::other("login context cannot rebind")) }
}
impl LoginCapture for FakeCapture {
    fn output_binding(&self) -> io::Result<OutputBinding> { Ok(self.binding.clone()) }
}
fn bound(context: &Context<FakeCapture>) -> bool {
    context.valid && context.capture.output_binding().ok().as_ref() == Some(&context.output)
}
fn context() -> (Context<FakeCapture>, CaptureGrant) {
    let pool = Arc::new(MemoryPool::default());
    let admission = pool.admit_session(None, CapacityCeiling::bounded(MAX_WORKING_BYTES),
        MAX_WORKING_BYTES, "login-pending-fixture").unwrap();
    admission.bind_contract(None).unwrap();
    let key = SourceKey::new(admission.session(), "login:monitor:fixture").unwrap();
    let mut plan = CapturePlan::new(key, CaptureGeneration::new(1));
    plan.add_bgra(CaptureOwner::SourcePixels, FrameSize::new(4, 4), 2, CapturePhases::ALL).unwrap();
    let grant = admission.authority().approve(plan).unwrap();
    let area = Area { left: 0, top: 0, width: 2, height: 2 };
    let output = OutputBinding { device: "fixture".into(), monitor: 1, area,
        rotation: 1, duplication_rotation: 1, generation: 1,
        size: FrameSize::new(2, 2), attached: true };
    let capture = FakeCapture { binding: output.clone(), samples: VecDeque::new(), last: None, calls: Vec::new() };
    (Context { capture, output, desk: area, valid: true, published: false, pending: None, sas: SasGuard::new() }, grant)
}
fn image(grant: &CaptureGrant, size: FrameSize, value: u8) -> CapturedImage {
    let mut builder = CaptureImageBuilder::new(size, FrameLimits::RESOURCE, grant, CaptureOwner::SourcePixels).unwrap();
    builder.pixels_mut().fill(value);
    builder.freeze()
}
fn pixels(frame: SourceFrame) -> CapturedImage {
    match frame { SourceFrame::Frame(image) => image, _ => panic!("expected the pending login image") }
}
#[test]
fn batches_task_b7_login_unsent_stills_survive_geometry_probes() {
    let (mut context, grant) = context();
    let first = image(&grant, context.output.size, 11);
    context.capture.samples.push_back(Sample::Image(first.clone()));
    context.probe_with(bound).unwrap();
    assert!(!context.published, "a geometry observation is no published image");
    context.probe_with(bound).unwrap();
    context.capture.samples.push_back(Sample::Paused);
    context.probe_with(bound).unwrap();
    assert!(context.pending.as_ref().unwrap().shares_pixels(&first));
    assert_eq!(grant.live_bytes().unwrap(), 16, "cache and pending alias one owner");
    let sent = pixels(context.capture_with(false, bound).unwrap());
    assert!(sent.shares_pixels(&first));
    assert!(context.published);
    assert!(context.pending.is_none());
    assert!(matches!(context.capture_with(false, bound).unwrap(), SourceFrame::Unchanged));

    let latest = image(&grant, context.output.size, 22);
    context.capture.samples.push_back(Sample::Image(latest.clone()));
    context.probe_with(bound).unwrap();
    context.probe_with(bound).unwrap();
    let sent = pixels(context.capture_with(false, bound).unwrap());
    assert!(sent.shares_pixels(&latest));
    assert_eq!(sent.image().data(), &[22; 16]);
    assert!(context.capture.calls.iter().all(|keyframe| !keyframe));
}
#[test]
fn batches_task_b7_login_new_sample_replaces_unsent_sample_without_allocating_aliases() {
    let (mut context, grant) = context();
    context.capture.samples.push_back(Sample::Image(image(&grant, context.output.size, 1)));
    context.probe_with(bound).unwrap();
    let latest = image(&grant, context.output.size, 2);
    context.capture.samples.push_back(Sample::Image(latest.clone()));
    context.probe_with(bound).unwrap();
    assert_eq!(grant.live_bytes().unwrap(), 16);
    assert!(!context.published);
    assert!(pixels(context.capture_with(false, bound).unwrap()).shares_pixels(&latest));
}
#[test]
fn batches_task_b7_login_probe_rejects_binding_size_and_capture_failure() {
    for failure in [Sample::Rebind, Sample::Fail] {
        let (mut context, grant) = context();
        context.capture.samples.push_back(Sample::Image(image(&grant, context.output.size, 1)));
        context.probe_with(bound).unwrap();
        context.capture.samples.push_back(failure);
        assert!(context.probe_with(bound).is_err());
        assert!(!context.valid && context.pending.is_none() && !context.published);
    }
    let (mut context, grant) = context();
    context.capture.samples.push_back(Sample::Image(image(&grant, FrameSize::new(3, 2), 3)));
    assert!(context.probe_with(bound).is_err());
    assert!(!context.valid && context.pending.is_none() && !context.published);
    let (mut context, _) = self::context();
    context.capture.binding.monitor = 2;
    assert!(context.probe_with(bound).is_err());
    assert!(context.capture.calls.is_empty(), "invalid initial identity cannot sample pixels");
}

struct Rebuild {
    budget: CaptureBudget,
    geometry: CaptureGeometry,
    live: Option<CaptureSlice>,
    attempts: usize,
}
impl Rebuild {
    fn restart(&mut self) -> io::Result<()> {
        self.attempts += 1;
        self.live = None;
        self.budget.suspend();
        self.budget.replace(self.geometry)?;
        self.live = Some(self.budget.claim_native()?);
        Ok(())
    }
    fn capture(&mut self, recovery: CaptureRecovery) -> io::Result<bool> {
        recovery.ensure_live(self.live.is_some(), || self.restart())
    }
}
#[test]
fn batches_task_b6_windows_rebuild_retries_after_real_owner_release() {
    for kind in [CaptureKind::Wgc, CaptureKind::Dxgi(BgraRotation::Identity)] {
        let old_size = FrameSize::new(64, 64);
        let new_size = FrameSize::new(256, 192);
        let old = CaptureGeometry { native: old_size, delivered: old_size, limits: FrameLimits::RESOURCE, kind };
        let new = CaptureGeometry { native: new_size, delivered: new_size, ..old };
        let key = SourceKey::new("windows-retry", "monitor:fixture").unwrap();
        let output = HostOutputPlan::new(HostContract::Legacy, old_size, HostCodecPlan::Tiles).unwrap().for_worker();
        let capacity = new.plan(key.clone(), CaptureGeneration::new(2), &output).unwrap().peak_bytes();
        assert!(capacity <= MAX_WORKING_BYTES);
        let pool = Arc::new(MemoryPool::default());
        let admission = pool.admit_session(None, CapacityCeiling::bounded(capacity), capacity, key.session()).unwrap();
        admission.bind_contract(None).unwrap();
        let grant = admission.authority().approve(old.plan(key.clone(), CaptureGeneration::new(1), &output).unwrap()).unwrap();
        let held_image = image(&grant, old_size, 1);
        let encoder = AdmittedTilesEncoder::new(old_size, FrameLimits::RESOURCE, grant.clone()).unwrap();
        let budget = CaptureBudget::activate(admission, key.clone(), old, output, grant.clone()).unwrap();
        let mut source = Rebuild { budget, geometry: new, live: None, attempts: 0 };
        let recovery = CaptureRecovery::active();
        assert_eq!(source.capture(recovery).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(source.budget.generation() > 1);
        assert!(source.live.is_none() && source.budget.grant().is_err());
        assert!(grant.is_retired().unwrap());
        assert!(grant.live_bytes().unwrap() >= bgra_bytes(old_size).unwrap() * 2);
        assert_eq!(held_image.image().data()[0], 1, "a retired image remains readable and funded");
        drop(held_image);
        assert_eq!(source.capture(recovery).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        drop(encoder);
        assert_eq!(grant.live_bytes().unwrap(), 0);
        assert!(source.capture(recovery).unwrap());
        assert_eq!(source.attempts, 3);
        assert!(source.live.is_some());
        source.budget.grant().unwrap().check_identity(&key, CaptureGeneration::new(source.budget.generation())).unwrap();
    }
}
#[test]
fn batches_task_b6_windows_pending_resume_can_be_paused_and_preserves_hard_errors() {
    let mut recovery = CaptureRecovery::active();
    recovery.activate(false, || Err(io::Error::from(io::ErrorKind::WouldBlock))).unwrap();
    recovery.request(false);
    assert!(!recovery.ensure_live(false, || panic!("a paused capture must not restart")).unwrap());
    recovery.request(true);
    assert!(recovery.ensure_live(false, || Ok(())).unwrap());
    assert_eq!(recovery.ensure_live(false, || Err(io::Error::from(io::ErrorKind::NotFound))).unwrap_err().kind(), io::ErrorKind::NotFound);
    assert_eq!(recovery.activate(false, || Err(io::Error::from(io::ErrorKind::PermissionDenied))).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}
