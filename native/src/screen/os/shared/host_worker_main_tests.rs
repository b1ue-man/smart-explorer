//! The error mapping of the host process (SCR-M5): a source the user
//! declined reaches the viewer as `no_display`, any other capture error as
//! `capture_failed`.
use super::*;
use crate::screen::core::host_backend::SourceRefusal;
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecKind, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;
use crate::screen::core::worker_wire::OpenRequest;
use crate::screen::os::synthetic_backend::SyntheticBackend;

/// A source whose every capture fails with `kind`.
struct FailingSource(io::ErrorKind);

impl ScreenSource for FailingSource {
    fn size(&self) -> FrameSize {
        FrameSize::new(640, 360)
    }

    fn capture(&mut self, _frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        Err(io::Error::new(self.0, "portal_denied: test"))
    }
}

/// The test-picture backend with a failing monitor source.
struct FailingBackend(io::ErrorKind);

impl ScreenHostBackend for FailingBackend {
    fn memory_facts(&self)->Option<crate::screen::core::frame_memory::MemoryFacts> {
        Some(crate::screen::core::frame_memory::MemoryFacts {available_physical:1<<40,available_commit:1<<40,available_address_space:1<<40})
    }
    fn platform(&self) -> HostPlatform {
        SyntheticBackend.platform()
    }

    fn graphical_session(&self) -> bool {
        true
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::tiles()]
    }

    fn input(&self) -> InputCapabilities {
        InputCapabilities::default()
    }

    fn limits(&self) -> Vec<String> {
        Vec::new()
    }

    fn windows(&self) -> io::Result<WindowList> {
        SyntheticBackend.windows()
    }

    fn open_source(
        &self,
        _source: &SourceId,
        _wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        Ok(Box::new(FailingSource(self.0)))
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        SyntheticBackend.open_encoder(choice, size)
    }

    fn inject(&self, _events: &[InputEvent]) -> Result<(), ControlBlock> {
        Err(ControlBlock::UnsupportedOnHost)
    }
}

fn worker(kind: io::ErrorKind) -> Worker {
    Worker {
        admitted:Some(None),memory:Some(Arc::new(crate::screen::core::frame_memory::MemoryPool::default())
            .reserve(None,crate::screen::core::frame_budget::MAX_WORKING_BYTES).unwrap()),
        backend: Arc::new(FailingBackend(kind)),
        source: None,
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
        control: false,
        capture_active: true,
        reference_generation: 0,
        family: None,
    }
}

fn request(size: FrameSize) -> StageRequest {
    StageRequest {
        frame_contract: None,
        visible_rects: None,
        frame_id: 1,
        keyframe: true,
        target_bps: 1_000_000,
        size,
        choice: CodecChoice {
            max_macroblocks_per_second: None,
            codec: CodecKind::Tiles,
            profile: None,
            encoder_hardware: false,
            decoder_hardware: false,
            max_size: FrameSize::MAX,
            max_pixels: crate::screen::core::pixel_convert::MAX_FRAME_PIXELS,
        },
        encoder_epoch: 1,
        fps: 30,
    }
}

fn code_of(reply: WorkerReply) -> ScreenErrorCode {
    match reply {
        WorkerReply::Failed { code, .. } => code,
        _ => panic!("the host process did not fail"),
    }
}

fn open(worker: &mut Worker) {
    let opened = worker.handle(WorkerRequest::Open(OpenRequest {
        source: SourceId::Monitor("portal".into()),
        wanted: None,
        control: false,
    }));
    assert!(matches!(opened, WorkerReply::Opened { .. }));
}

#[test]
fn screen_task_worker_maps_permission_denied_to_no_display() {
    let mut worker = worker(io::ErrorKind::PermissionDenied);
    open(&mut worker);
    let size = FrameSize::new(320, 180);
    let produced = worker.handle(WorkerRequest::Produce(request(size)));
    assert_eq!(code_of(produced), ScreenErrorCode::NoDisplay);
    let raw = worker.handle(WorkerRequest::Capture { frame_id: 2,frame_contract:None });
    assert_eq!(code_of(raw), ScreenErrorCode::NoDisplay);
}

#[test]
fn screen_task_worker_keeps_other_capture_errors_as_capture_failed() {
    let mut worker = worker(io::ErrorKind::BrokenPipe);
    open(&mut worker);
    let produced = worker.handle(WorkerRequest::Produce(request(FrameSize::new(320, 180))));
    assert_eq!(code_of(produced), ScreenErrorCode::CaptureFailed);
}

/// Input reaches the platform backend only in a session opened for control
/// (SCR-M6); the frames survive the wire unchanged.
#[test]
fn screen_task_worker_input_needs_control_session() {
    use crate::screen::core::worker_wire::{BoundInput, WorkerReply as Reply, WorkerRequest as Request};
    let events = vec![InputEvent::Text { text: "x".into() }];
    let input = BoundInput { events, reference_generation: 0, presented: false };
    let mut viewing = worker(io::ErrorKind::BrokenPipe);
    open(&mut viewing);
    assert_eq!(
        viewing.handle(Request::Input(input.clone())),
        Reply::InputDone {
            blocked: Some(ControlBlock::ControlUnavailable),
            legacy_reconnect: false,
        }
    );
    let mut controlling = worker(io::ErrorKind::BrokenPipe);
    let opened = controlling.handle(Request::Open(OpenRequest {
        source: SourceId::Monitor("portal".into()),
        wanted: None,
        control: true,
    }));
    assert!(matches!(opened, Reply::Opened { .. }));
    // The test backend refuses input: its reason travels back.
    assert_eq!(
        controlling.handle(Request::Input(input.clone())),
        Reply::InputDone {
            blocked: Some(ControlBlock::UnsupportedOnHost),
            legacy_reconnect: false,
        }
    );
    let (tag, payload) = Request::Input(input.clone()).encode().unwrap();
    assert_eq!(tag, 10, "private input carries its capture reference");
    assert!(Request::decode(5, &payload).is_err(), "unbound private input is refused");
    assert_eq!(
        Request::decode(tag, &payload).unwrap(),
        Request::Input(input)
    );
    let done = Reply::InputDone {
        blocked: Some(ControlBlock::ElevatedWindow),
        legacy_reconnect: true,
    };
    let (tag, payload) = done.encode().unwrap();
    assert_eq!(Reply::decode(tag, &payload).unwrap(), done);
}

struct SnapshotSource;
impl ScreenSource for SnapshotSource {
    fn size(&self)->FrameSize {FrameSize::new(640,360)}
    fn capture(&mut self,_id:u64,_key:bool)->io::Result<SourceFrame> {
        let mut image=crate::screen::core::pixel_convert::BgraImage::new(self.size())?;
        for pixel in image.data_mut().chunks_exact_mut(4) {pixel.copy_from_slice(&[1,2,3,255]);}
        Ok(SourceFrame::Frame(image))
    }
}
#[test]
fn batches_task_b6_first_worker_snapshot_admits_contract_and_prevents_later_growth() {
    use crate::screen::core::frame_budget::{FrameCapabilities,FrameContract};
    let mut worker=worker(io::ErrorKind::BrokenPipe);worker.admitted=None;worker.memory=None;
    worker.source=Some(Box::new(SnapshotSource));
    let mut caps=FrameCapabilities::admitted(FrameSize::new(640,360),4*1024*1024).unwrap();caps.hybrid_regions=false;
    let contract=FrameContract::negotiate(Some(caps),Some(caps)).unwrap();
    let capture=WorkerRequest::Capture {frame_id:1,frame_contract:contract};
    let (tag,payload)=capture.encode().unwrap();
    let raw=worker.handle(WorkerRequest::decode(tag,&payload).unwrap());
    let WorkerReply::Raw(image)=raw else {panic!("first snapshot must capture after admission");};
    let limits=contract.unwrap().limits;
    assert_eq!(image.size(),limits.fit(FrameSize::new(640,360)));
    assert_eq!(worker.memory.as_ref().unwrap().bytes(),limits.working_bytes);
    let mut next=request(image.size());next.frame_contract=contract;next.frame_id=2;
    assert!(matches!(worker.handle(WorkerRequest::Produce(next.clone())),WorkerReply::Frame(StageFrame::Encoded {..})));
    let bigger=FrameCapabilities::admitted(FrameSize::new(640,360),8*1024*1024).unwrap();
    let bigger=FrameContract::negotiate(Some(bigger),Some(bigger)).unwrap();
    assert!(matches!(worker.handle(WorkerRequest::Capture {frame_id:3,frame_contract:bigger}),WorkerReply::Failed {..}));
    next.frame_contract=bigger;
    assert!(matches!(worker.handle(WorkerRequest::Produce(next)),WorkerReply::Failed {..}));
    assert_eq!(worker.memory.as_ref().unwrap().bytes(),limits.working_bytes);
    // Missing optional field keeps the original legacy request semantics.
    let legacy=WorkerRequest::decode(tag,br#"{"frame_id":4}"#).unwrap();
    assert!(matches!(&legacy,WorkerRequest::Capture {frame_contract:None,..}));
    assert!(matches!(worker.handle(legacy),WorkerReply::Failed {..}));
}
