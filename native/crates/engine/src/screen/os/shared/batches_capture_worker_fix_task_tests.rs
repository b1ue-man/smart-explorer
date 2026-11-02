//! Deterministic worker turnover/locking fixtures for the shared remote suite.
use super::*;
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SourceKey},
    capture_grant::DelegatedCaptureCredit,
    capture_image::CaptureImageBuilder,
    capture_plan::{CapturePhases, OWNER_COUNT},
    capture_test_support,
    host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan},
    pixel_convert::FrameSize,
    session_state::SessionState,
    worker_capture_wire::{CreditWire, Reduction, REDUCED},
};
use std::sync::{
    atomic::{AtomicU64, AtomicUsize},
    Mutex,
};

struct Recording(Arc<Mutex<Vec<u8>>>);
impl io::Write for Recording {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct SourceState {
    grant: Mutex<CaptureGrant>,
    reference: AtomicU64,
    reads: AtomicUsize,
    retire_at: AtomicUsize,
    change: AtomicBool,
    during_capture: AtomicBool,
    layout: AtomicU64,
    pause: Mutex<Option<SessionState>>,
    failure: Mutex<Option<io::ErrorKind>>,
}
struct Source(Arc<SourceState>);
impl ScreenSource for Source {
    fn size(&self) -> FrameSize {
        FrameSize::new(32, 18)
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.0.grant.lock().unwrap().clone())
    }
    fn family_layout_revision(&self) -> Option<u64> {
        let revision = self.0.layout.load(Ordering::SeqCst);
        (revision != 0).then_some(revision)
    }
    fn reference_generation(&self) -> u64 {
        let value = self.0.reference.load(Ordering::SeqCst);
        let read = self.0.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if read == self.0.retire_at.load(Ordering::SeqCst) {
            if self.0.change.load(Ordering::SeqCst) {
                self.0.reference.store(value + 1, Ordering::SeqCst);
            }
            self.0.grant.lock().unwrap().retire().unwrap();
        }
        value
    }
    fn capture(&mut self, _frame: u64, _key: bool) -> io::Result<SourceFrame> {
        let grant = self.0.grant.lock().unwrap().clone();
        let mut image = CaptureImageBuilder::new(
            self.size(),
            FrameLimits::LEGACY,
            &grant,
            CaptureOwner::SourcePixels,
        )?;
        image.pixels_mut().fill(31);
        self.0.layout.store(3, Ordering::SeqCst);
        if self.0.during_capture.swap(false, Ordering::SeqCst) {
            self.0.reference.fetch_add(1, Ordering::SeqCst);
            grant.retire()?;
        }
        if let Some(state) = *self.0.pause.lock().unwrap() {
            return Ok(SourceFrame::Paused(state));
        }
        if let Some(kind) = *self.0.failure.lock().unwrap() {
            return Err(io::Error::new(kind, "fixture source permission"));
        }
        Ok(SourceFrame::Frame(image.freeze()))
    }
}
fn plan(source: SourceKey, generation: u64) -> CapturePlan {
    let size = FrameSize::new(32, 18);
    let mut plan = CapturePlan::new(source, CaptureGeneration::new(generation));
    plan.add_bgra(CaptureOwner::SourcePixels, size, 3, CapturePhases::ALL)
        .unwrap();
    HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles)
        .unwrap()
        .for_worker()
        .append_to(&mut plan, HostInputPixels::BorrowedSource)
        .unwrap();
    plan
}
fn import(
    remote: &dispatch::RemoteAuthority,
    source: SourceKey,
    generation: u64,
    id: u64,
) -> CaptureGrant {
    let plan = plan(source, generation);
    let credit = CreditWire {
        id,
        source: plan.source().source().into(),
        generation,
        total: plan.peak_bytes(),
        owners: plan.owner_limits(),
    };
    remote.import(plan, credit).unwrap()
}
fn new_worker(
    retire_at: usize,
) -> (
    Worker,
    Arc<SourceState>,
    dispatch::Output,
    Arc<Mutex<Vec<u8>>>,
) {
    let admission = capture_test_support::session(None);
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let output: dispatch::Output = Arc::new(Mutex::new(Box::new(Recording(recorded.clone()))));
    let remote = dispatch::RemoteAuthority::new(output.clone());
    remote.bind_session(admission.session()).unwrap();
    let grant = import(
        &remote,
        SourceKey::new(admission.session(), "pixels").unwrap(),
        1,
        23,
    );
    let source = Arc::new(SourceState {
        grant: Mutex::new(grant),
        reference: AtomicU64::new(7),
        reads: AtomicUsize::new(0),
        retire_at: AtomicUsize::new(retire_at),
        change: AtomicBool::new(true),
        during_capture: AtomicBool::new(false),
        layout: AtomicU64::new(0),
        pause: Mutex::new(None),
        failure: Mutex::new(None),
    });
    let worker = Worker {
        admitted: Some(None),
        backend: Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
        source: Some(Box::new(Source(source.clone()))),
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
        control: false,
        capture_active: true,
        reference_generation: 7,
        family: None,
        prepared: None,
        planned: None,
        capture_session: Some(admission),
        remote,
        release_sink: None,
    };
    (worker, source, output, recorded)
}
fn request(raw: bool, frame: u64) -> WorkerRequest {
    if raw {
        return WorkerRequest::Capture {
            frame_id: frame,
            frame_contract: None,
        };
    }
    WorkerRequest::Produce(StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id: frame,
        keyframe: true,
        target_bps: 1_000_000,
        size: FrameSize::new(32, 18),
        choice: CodecChoice {
            max_macroblocks_per_second: None,
            codec: crate::screen::core::negotiate::CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::MAX,
            max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
        },
        encoder_epoch: 1,
        fps: 30,
    })
}
fn waiting(reply: &PreparedReply, raw: bool) {
    assert!(reply.bytes.is_none());
    let WorkerReply::Reference {
        generation, reply, ..
    } = &reply.reply
    else {
        panic!("missing waiting reference");
    };
    assert_eq!(*generation, 8);
    assert!(match reply.as_ref() {
        WorkerReply::Frame(StageFrame::Paused(SessionState::Starting)) => raw,
        WorkerReply::Frame(StageFrame::Unchanged { .. }) => !raw,
        _ => false,
    });
}

#[test]
fn batches_task_capture_integration_worker_retired_transport_waits_then_answers_the_next_picture() {
    for raw in [true, false] {
        // RAW samples before/after capture; PRODUCE also samples around capture.
        let (mut worker, source, output, recorded) = new_worker(if raw { 2 } else { 4 });
        let reply = worker.prepare_reply(request(raw, 1)).unwrap();
        waiting(&reply, raw);
        reply.write(&output).unwrap();
        drop(reply);
        assert!(source.grant.lock().unwrap().is_retired().unwrap());
        source.retire_at.store(0, Ordering::SeqCst);
        let key = source.grant.lock().unwrap().source().clone();
        *source.grant.lock().unwrap() = import(&worker.remote, key, 2, 24);
        let next = worker.prepare_reply(request(raw, 2)).unwrap();
        assert!(
            next.bytes.is_some(),
            "the same worker captures after new admission"
        );
        let WorkerReply::Reference { generation, .. } = &next.reply else {
            panic!("missing picture reference");
        };
        assert_eq!(*generation, 8);
        next.write(&output).unwrap();
        assert!(!recorded.lock().unwrap().is_empty());
    }
}

#[test]
fn batches_task_capture_integration_worker_turnover_during_capture_preserves_pause_and_liveness() {
    for raw in [true, false] {
        let (mut worker, source, output, _) = new_worker(0);
        source.during_capture.store(true, Ordering::SeqCst);
        let reply = worker.prepare_reply(request(raw, 1)).unwrap();
        waiting(&reply, raw);
        reply.write(&output).unwrap();
        let next = worker
            .prepare_reply(WorkerRequest::SetCaptureActive(false))
            .unwrap();
        assert!(
            matches!(next.reply,WorkerReply::Reference {reply,..} if matches!(*reply,WorkerReply::CaptureActive(false)))
        );
    }
}

#[test]
fn batches_task_capture_integration_worker_turnover_preserves_native_pause_reason() {
    for raw in [true, false] {
        let (mut worker, source, output, _) = new_worker(0);
        source.during_capture.store(true, Ordering::SeqCst);
        *source.pause.lock().unwrap() = Some(SessionState::WindowMinimized);
        let reply = worker.prepare_reply(request(raw, 1)).unwrap();
        assert!(reply.bytes.is_none());
        assert!(
            matches!(&reply.reply,WorkerReply::Reference {generation:8,reply,..}
            if matches!(reply.as_ref(),WorkerReply::Frame(StageFrame::Paused(SessionState::WindowMinimized))))
        );
        reply.write(&output).unwrap();

        let (mut worker, source, output, _) = new_worker(0);
        source.during_capture.store(true, Ordering::SeqCst);
        *source.failure.lock().unwrap() = Some(io::ErrorKind::PermissionDenied);
        let reply = worker.prepare_reply(request(raw, 2)).unwrap();
        assert!(
            matches!(&reply.reply,WorkerReply::Reference {generation:8,reply,..}
            if matches!(reply.as_ref(),WorkerReply::Failed {code:ScreenErrorCode::NoDisplay,..}))
        );
        reply.write(&output).unwrap();
    }
}

#[test]
fn batches_task_capture_integration_worker_prepared_picture_keeps_its_captured_reference() {
    let (mut worker, source, output, recorded) = new_worker(0);
    let reply = worker.prepare_reply(request(true, 1)).unwrap();
    assert!(
        matches!(
            &reply.reply,
            WorkerReply::Reference {
                family_layout_revision: Some(3),
                ..
            }
        ),
        "a newly published layout belongs to this capture, not its preceding frame"
    );
    let live = source.grant.lock().unwrap().live_bytes().unwrap();
    source.reference.store(8, Ordering::SeqCst);
    source.grant.lock().unwrap().retire().unwrap();
    reply.write(&output).unwrap();
    assert!(live > 0);
    let parent = capture_test_support::grant(FrameSize::new(32, 18), None);
    let bytes = recorded.lock().unwrap().clone();
    let mut input = std::io::Cursor::new(bytes);
    let mut received = None;
    while let Some((tag, length)) = capture_transport::read_header(&mut input).unwrap() {
        if tag == crate::screen::core::worker_capture_wire::OWNED_REFERENCE {
            let (_, grant, payload) =
                capture_transport::read_owned(&mut input, length, None, |id| {
                    assert_eq!(id, 23);
                    Ok(parent.clone())
                })
                .unwrap();
            received = Some(capture_transport::decode_owned(&payload, &grant, None).unwrap());
        } else {
            capture_transport::read_control(&mut input, tag, length, None).unwrap();
        }
    }
    assert!(
        matches!(received,Some(WorkerReply::Reference {generation:7,reply,..}) if matches!(*reply,WorkerReply::Raw(_)))
    );
    drop(reply);
    let next = worker
        .prepare_reply(WorkerRequest::SetCaptureActive(false))
        .unwrap();
    assert!(matches!(
        next.reply,
        WorkerReply::Reference { generation: 8, .. }
    ));
}

struct BlockingWriter {
    entered: Option<std::sync::mpsc::Sender<()>>,
    proceed: std::sync::mpsc::Receiver<()>,
    fail: bool,
}
impl io::Write for BlockingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap();
            self.proceed.recv_timeout(Duration::from_secs(2)).unwrap();
            if self.fail {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct LastOwner(Arc<AtomicUsize>);
impl Drop for LastOwner {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn batches_task_capture_integration_worker_write_and_retirement_finish_without_lock_cycle() {
    for fail in [false, true] {
        let (entered, writing) = std::sync::mpsc::channel();
        let (resume, proceed) = std::sync::mpsc::channel();
        let output: dispatch::Output = Arc::new(Mutex::new(Box::new(BlockingWriter {
            entered: Some(entered),
            proceed,
            fail,
        })));
        let (notifying, observed) = std::sync::mpsc::channel();
        let snapshots = Arc::new(Mutex::new(Vec::<[u64; OWNER_COUNT]>::new()));
        let observed_snapshots = snapshots.clone();
        let notifications = output.clone();
        let observer = Arc::new(move |owners| {
            notifying.send(()).unwrap();
            let mut output = notifications.lock().unwrap();
            let payload =
                crate::screen::core::wire::to_json(&Reduction { id: 23, owners }).unwrap();
            crate::screen::core::worker_wire::write_message(&mut *output, REDUCED, &payload)
                .unwrap();
            observed_snapshots.lock().unwrap().push(owners);
        });
        let ended = Arc::new(AtomicUsize::new(0));
        let source = SourceKey::new("capture-fixture", "pixels").unwrap();
        let plan = plan(source.clone(), 1);
        let credit = DelegatedCaptureCredit::from_verified_parent_observed(
            source,
            CaptureGeneration::new(1),
            plan.peak_bytes(),
            plan.owner_limits(),
            LastOwner(ended.clone()),
            observer,
        )
        .unwrap();
        let grant = CaptureGrant::from_delegated(plan, credit).unwrap();
        let image = CaptureImageBuilder::new(
            FrameSize::new(32, 18),
            FrameLimits::LEGACY,
            &grant,
            CaptureOwner::SourcePixels,
        )
        .unwrap()
        .freeze();
        let reply = Reference {
            generation: 7,
            regions: true,
            limits: FrameLimits::LEGACY,
            family_layout_revision: None,
        }
        .wrap(WorkerReply::Raw(image));
        let bytes = capture_transport::prepare_owned(&reply, 23, &grant)
            .unwrap()
            .unwrap();
        let prepared = PreparedReply {
            reply,
            bytes: Some(bytes),
        };
        let (done, finished) = std::sync::mpsc::channel();
        let sender = std::thread::spawn(move || {
            let sent = prepared.write(&output);
            drop(prepared);
            done.send(sent).unwrap();
        });
        writing.recv_timeout(Duration::from_secs(2)).unwrap();
        let retiring = grant.clone();
        let retirement = std::thread::spawn(move || retiring.retire());
        observed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(grant.live_bytes().unwrap() > 0);
        assert_eq!(ended.load(Ordering::SeqCst), 0);
        resume.send(()).unwrap();
        let sent = finished
            .recv_timeout(Duration::from_secs(2))
            .expect("picture write/drop deadlocked with retirement");
        assert_eq!(sent.is_err(), fail);
        sender.join().unwrap();
        retirement.join().unwrap().unwrap();
        assert_eq!(grant.live_bytes().unwrap(), 0);
        let snapshots = snapshots.lock().unwrap();
        assert!(!snapshots.is_empty());
        for pair in snapshots.windows(2) {
            assert!(pair[1]
                .iter()
                .zip(pair[0])
                .all(|(next, previous)| *next <= previous));
        }
        assert_eq!(snapshots.last(), Some(&[0; OWNER_COUNT]));
        assert_eq!(ended.load(Ordering::SeqCst), 0);
        drop(grant);
        assert_eq!(ended.load(Ordering::SeqCst), 1);
    }
}
