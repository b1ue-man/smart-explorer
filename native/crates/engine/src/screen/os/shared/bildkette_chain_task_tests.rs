//! Task suite „Bildkette“ BK3 (plan 2026-11-14 B-27 to B-29): host-process
//! stream with a counting source and delayed tile encoder.
use super::*;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureOwner, CapturePlan, SourceKey,
};
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_plan::CapturePhases;
use crate::screen::core::capture_test_support;
use crate::screen::core::capture_transport;
use crate::screen::core::codec_registry::OpenedEncoder;
use crate::screen::core::host_backend::CapturedEncodedFrame;
use crate::screen::core::host_codec_adapters::{
    HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan,
};
use crate::screen::core::negotiate::CodecKind;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::worker_capture_wire::{CreditWire, PreparedInfoWire};
use crate::screen::core::worker_wire::{self, BoundInput, StreamMark, STREAMED, STREAM_VERSION};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc;

const SIZE: FrameSize = FrameSize::new(32, 18);
/// The test clock: one capture per 10 ms.
const CADENCE: u32 = 100;
const GENERATION: u64 = 7;

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

/// What the test source saw: every capture call and its time.
#[derive(Default)]
struct CaptureProbe {
    count: AtomicU64,
    times: Mutex<Vec<Instant>>,
}

/// Fills every picture with the number of its capture call.
struct CountingSource {
    probe: Arc<CaptureProbe>,
    grant: CaptureGrant,
}

impl ScreenSource for CountingSource {
    fn size(&self) -> FrameSize {
        SIZE
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        Some(self.grant.clone())
    }
    fn reference_generation(&self) -> u64 {
        GENERATION
    }
    fn capture(&mut self, _frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        let mut image = CaptureImageBuilder::new(
            SIZE,
            FrameLimits::LEGACY,
            &self.grant,
            CaptureOwner::SourcePixels,
        )?;
        let count = self.probe.count.fetch_add(1, Ordering::SeqCst) + 1;
        image.pixels_mut().fill(count as u8);
        self.probe.times.lock().unwrap().push(Instant::now());
        Ok(SourceFrame::Frame(image.freeze()))
    }
}

/// When each encoding ran, and whether one runs now.
#[derive(Default)]
struct EncodeProbe {
    running: AtomicBool,
    spans: Mutex<Vec<(Instant, Instant)>>,
}

/// The registry's encoder, `delay` slower per picture.
struct SlowEncoder {
    inner: Box<dyn ScreenEncoder>,
    delay: Duration,
    probe: Arc<EncodeProbe>,
}

impl ScreenEncoder for SlowEncoder {
    fn encode_captured(
        &mut self,
        frame: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let started = Instant::now();
        self.probe.running.store(true, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        let encoded = self.inner.encode_captured(frame, request);
        self.probe
            .spans
            .lock()
            .unwrap()
            .push((started, Instant::now()));
        self.probe.running.store(false, Ordering::SeqCst);
        encoded
    }
    fn name(&self) -> String {
        self.inner.name()
    }
    fn size(&self) -> FrameSize {
        self.inner.size()
    }
    fn needs_refresh(&self) -> bool {
        self.inner.needs_refresh()
    }
    fn encode(
        &mut self,
        frame: &crate::screen::core::pixel_convert::BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<crate::screen::core::host_backend::EncodedFrame> {
        self.inner.encode(frame, keyframe, target_bps)
    }
}

struct SlowOpener {
    delay: Duration,
    probe: Arc<EncodeProbe>,
}

impl EncoderOpener for SlowOpener {
    fn open(
        &self,
        request: &StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        let OpenedEncoder {
            encoder,
            factory,
            hardware,
        } = Registry.open(request, refused, grant)?;
        Ok(OpenedEncoder {
            encoder: Box::new(SlowEncoder {
                inner: encoder,
                delay: self.delay,
                probe: self.probe.clone(),
            }),
            factory,
            hardware,
        })
    }
}

struct Rig {
    stream: Arc<Stream>,
    worker: Arc<Mutex<Worker>>,
    output: dispatch::Output,
    remote: Arc<dispatch::RemoteAuthority>,
    recorded: Arc<Mutex<Vec<u8>>>,
    captures: Arc<CaptureProbe>,
    encodes: Arc<EncodeProbe>,
}

fn plan(source: SourceKey) -> CapturePlan {
    let mut plan = CapturePlan::new(source, CaptureGeneration::new(1));
    plan.add_bgra(CaptureOwner::SourcePixels, SIZE, 3, CapturePhases::ALL)
        .unwrap();
    HostOutputPlan::new(HostContract::Legacy, SIZE, HostCodecPlan::Tiles)
        .unwrap()
        .for_worker()
        .append_to(&mut plan, HostInputPixels::BorrowedSource)
        .unwrap();
    plan
}

/// A worker with the counting source and a stream whose encoder takes
/// `delay` per picture; `control` lets the session inject input.
fn rig(delay: Duration, control: bool) -> Rig {
    let admission = capture_test_support::session(None);
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let output: dispatch::Output = Arc::new(Mutex::new(Box::new(Recording(recorded.clone()))));
    let remote = dispatch::RemoteAuthority::new(output.clone());
    remote.bind_session(admission.session()).unwrap();
    let plan = plan(SourceKey::new(admission.session(), "pixels").unwrap());
    let credit = CreditWire {
        id: 41,
        source: plan.source().source().into(),
        generation: 1,
        total: plan.peak_bytes(),
        owners: plan.owner_limits(),
    };
    let grant = remote.import(plan, credit).unwrap();
    let captures = Arc::new(CaptureProbe::default());
    let worker = Arc::new(Mutex::new(Worker {
        admitted: Some(None),
        backend: Arc::new(crate::screen::os::synthetic_backend::SyntheticBackend),
        source: Some(Box::new(CountingSource {
            probe: captures.clone(),
            grant,
        })),
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
        control,
        capture_active: true,
        reference_generation: GENERATION,
        family: None,
        prepared: None,
        planned: None,
        capture_session: Some(admission),
        remote: remote.clone(),
        release_sink: None,
    }));
    let encodes = Arc::new(EncodeProbe::default());
    let stream = Stream::new(
        worker.clone(),
        output.clone(),
        remote.clone(),
        Arc::new(SlowOpener {
            delay,
            probe: encodes.clone(),
        }),
    );
    Rig {
        stream,
        worker,
        output,
        remote,
        recorded,
        captures,
        encodes,
    }
}

fn request() -> StageRequest {
    StageRequest {
        frame_id: 1,
        frame_contract: None,
        visible_rects: None,
        keyframe: false,
        target_bps: 1_000_000,
        size: SIZE,
        choice: CodecChoice {
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::MAX,
            max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
            max_macroblocks_per_second: None,
        },
        encoder_epoch: 1,
        fps: CADENCE,
    }
}

fn order(granted: u64, ahead: u32, key_serial: u64) -> StreamOrder {
    StreamOrder {
        request: request(),
        granted,
        ahead,
        key_serial,
        cadence_fps: CADENCE,
        recover_serial: 0,
        last_good: 0,
    }
}

/// Tags and small payloads of every complete frame written so far.
fn written(recorded: &Mutex<Vec<u8>>) -> Vec<(u8, Vec<u8>)> {
    let bytes = recorded.lock().unwrap().clone();
    let mut input = io::Cursor::new(bytes);
    let mut frames = Vec::new();
    while let Ok(Some((tag, length))) = capture_transport::read_header(&mut input) {
        let mut payload = vec![0; length];
        if io::Read::read_exact(&mut input, &mut payload).is_err() {
            break;
        }
        frames.push((tag, payload));
    }
    frames
}

fn marks(recorded: &Mutex<Vec<u8>>) -> Vec<StreamMark> {
    written(recorded)
        .into_iter()
        .filter(|(tag, _)| *tag == STREAMED)
        .map(|(_, payload)| StreamMark::decode(&payload).unwrap())
        .collect()
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done()
}

/// B-27, requirement 2: the next capture runs while the previous picture
/// is encoded; it does not wait for the end of the encoding.
#[test]
fn bildkette_task_chain_capture_overlaps_encoding() {
    let rig = rig(Duration::from_millis(120), false);
    rig.stream.order(order(1, 1, 1));
    assert!(wait_until(Duration::from_secs(10), || {
        rig.encodes.spans.lock().unwrap().len() >= 2
    }));
    let spans = rig.encodes.spans.lock().unwrap().clone();
    let captures = rig.captures.times.lock().unwrap().clone();
    let overlapped = spans
        .iter()
        .any(|(started, ended)| captures.iter().any(|at| *at > *started && *at < *ended));
    assert!(overlapped, "no capture ran while a picture was encoded");
    assert_eq!(marks(&rig.recorded).len(), 1, "one credit, one answer");
    rig.stream.stop();
}

/// B-27: without credit nothing encodes; one credit + ahead encodes two.
#[test]
fn bildkette_task_chain_credit_bounds_the_pictures_in_flight() {
    let rig = rig(Duration::ZERO, false);
    rig.stream.order(order(0, 0, 0));
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.encodes.spans.lock().unwrap().is_empty());
    assert!(marks(&rig.recorded).is_empty());
    {
        let state = rig.stream.shared().lock();
        assert!(state.slot.is_some(), "the newest capture waits");
        assert!(state.held.is_none());
    }
    assert_eq!(rig.captures.count.load(Ordering::SeqCst), 1);

    rig.stream.order(order(1, 1, 1));
    assert!(wait_until(Duration::from_secs(5), || {
        rig.stream.shared().lock().held.is_some()
    }));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(rig.encodes.spans.lock().unwrap().len(), 2);
    let answers = marks(&rig.recorded);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].answer, 1);
    assert_ne!(answers[0].frame_id, 0, "the answer is a picture");

    // The next credit is answered at once with the picture held for it.
    rig.stream.order(order(2, 0, 1));
    assert!(wait_until(Duration::from_secs(5), || {
        marks(&rig.recorded).len() == 2
    }));
    assert_eq!(marks(&rig.recorded)[1].answer, 2);
    rig.stream.stop();
}

/// B-27, requirement 2: between the stages lies at most one picture, the
/// newest; each replaced one counts as dropped before encoding.
#[test]
fn bildkette_task_chain_newest_picture_replaces_the_waiting_one() {
    let rig = rig(Duration::from_millis(400), false);
    rig.stream.order(order(1, 0, 1));
    assert!(wait_until(Duration::from_secs(5), || {
        rig.encodes.running.load(Ordering::SeqCst)
    }));
    std::thread::sleep(Duration::from_millis(120));
    let checked = wait_until(Duration::from_secs(2), || {
        let state = rig.stream.shared().lock();
        if state.capturing || state.dropped < 2 {
            return false;
        }
        let newest = rig.captures.count.load(Ordering::SeqCst);
        let Some(waiting) = state.slot.as_ref() else {
            return false;
        };
        let crate::screen::core::capture_picture::Picture::Cpu(image) = &waiting.image else {
            panic!("the test source delivers CPU pictures");
        };
        assert_eq!(
            u64::from(image.image().data()[0]),
            newest % 256,
            "the waiting picture is the newest capture"
        );
        true
    });
    assert!(checked, "no picture waited while the encoder ran");
    rig.stream.stop();
}

/// B-28: input reaches the source while a picture is encoded; it does not
/// wait for the encoding.
#[test]
fn bildkette_task_chain_input_arrives_during_a_long_encoding() {
    let rig = rig(Duration::from_millis(1500), true);
    let lane = super::super::input::InputLane::start(
        rig.worker.clone(),
        rig.output.clone(),
        rig.remote.clone(),
    )
    .unwrap();
    lane.publish(&rig.worker.lock().unwrap());
    let (requests, incoming) = mpsc::sync_channel(1);
    let (rest, router) =
        super::super::input::route(incoming, lane.clone(), rig.stream.clone()).unwrap();
    requests
        .send(Ok(WorkerRequest::Stream(order(1, 0, 1))))
        .unwrap();
    assert!(wait_until(Duration::from_secs(5), || {
        rig.encodes.running.load(Ordering::SeqCst)
    }));
    let sent = Instant::now();
    requests
        .send(Ok(WorkerRequest::Input(BoundInput {
            events: Vec::new(),
            reference_generation: GENERATION,
            presented: true,
        })))
        .unwrap();
    let answered = wait_until(Duration::from_millis(500), || {
        written(&rig.recorded)
            .iter()
            .any(|(tag, _)| *tag == worker_wire::INPUT_DONE)
    });
    assert!(answered, "input waited {:?}", sent.elapsed());
    assert!(
        rig.encodes.running.load(Ordering::SeqCst),
        "the encoding still ran when the input was answered"
    );
    drop(requests);
    router.join().unwrap();
    assert!(
        rest.try_recv().is_err(),
        "input and orders never reach the request loop"
    );
    lane.stop();
    rig.stream.stop();
}

/// Stream wire: order and mark round-trip, ahead <= 1, version 0 -> PRODUCE.
#[test]
fn bildkette_task_chain_stream_wire_and_versions() {
    let order = order(9, 1, 3);
    let (tag, payload) = WorkerRequest::Stream(order.clone()).encode().unwrap();
    assert_eq!(tag, worker_wire::STREAM);
    assert_eq!(
        WorkerRequest::decode(tag, &payload).unwrap(),
        WorkerRequest::Stream(order.clone())
    );
    let mut greedy = order;
    greedy.ahead = 2;
    let (tag, payload) = WorkerRequest::Stream(greedy).encode().unwrap();
    assert!(WorkerRequest::decode(tag, &payload).is_err());

    let mark = StreamMark {
        answer: 4,
        frame_id: 77,
        capture_us: 1_500,
        encode_us: 9_000,
        dropped: 2,
        ..StreamMark::default()
    };
    assert_eq!(StreamMark::decode(&mark.encode()).unwrap(), mark);
    assert_eq!(worker_wire::tag_limit(STREAMED), Some(StreamMark::BYTES));

    let info = PreparedInfoWire {
        source: "pixels".into(),
        size: Some(SIZE),
        limits: FrameLimits::LEGACY,
        regions: true,
        state: SessionState::Starting,
        generation: 1,
        stream: STREAM_VERSION,
    };
    let mut json = serde_json::to_value(&info).unwrap();
    json.as_object_mut().unwrap().remove("stream");
    let older: PreparedInfoWire = serde_json::from_value(json).unwrap();
    assert_eq!(older.stream, 0, "an older process keeps PRODUCE");
    assert!(STREAM_VERSION >= 1);
}

/// B-29: both stages raise their thread priority, on Windows to
/// `THREAD_PRIORITY_HIGHEST`, and nothing raises the process class.
#[test]
fn bildkette_task_chain_stage_threads_raise_their_priority_on_windows() {
    let windows = include_str!("../windows/worker_thread.rs");
    assert!(windows.contains("SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST)"));
    assert!(!windows.contains("SetPriorityClass("));
    for stage in [
        include_str!("host_worker_capture_stage.rs"),
        include_str!("host_worker_encode.rs"),
    ] {
        assert!(stage.contains("crate::screen::os::worker_thread::raise_priority();"));
    }
}
