//! The bound worker source and its retained release/input endpoint.
use super::*;

/// A source captured and encoded in its host process.
pub struct WorkerSource {
    pub(super) facts: Arc<SourceFacts>,
    pub(super) link: SharedLink,
    pub(super) size: FrameSize,
    pub(super) broker: Arc<capture_broker::CaptureBroker>,
}
struct WorkerInputSink(SharedLink, Arc<SourceFacts>);
pub(super) fn input_sink(
    link: SharedLink,
    facts: Arc<SourceFacts>,
) -> Arc<dyn crate::screen::core::host_backend::ScreenInputSink> {
    Arc::new(WorkerInputSink(link, facts))
}
impl WorkerInputSink {
    fn family(
        &self,
        command: crate::screen::core::worker_family::FamilyCommand,
        active: Option<Arc<AtomicBool>>,
    ) -> io::Result<crate::screen::core::worker_family::FamilyReply> {
        let mut link = self
            .0
            .lock()
            .map_err(|_| io::Error::other("family_worker_link_poisoned"))?;
        if active.is_some_and(|active| !active.load(Ordering::Acquire)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "family_request_cancelled",
            ));
        }
        let limit = match &command {
            crate::screen::core::worker_family::FamilyCommand::Input(input) => {
                super::input_limit::input_limit(&input.events)
            }
            _ => INPUT_TIMEOUT,
        };
        match link
            .ask(&WorkerRequest::Family(command), limit)
            .map_err(|failure| failure.error)?
        {
            WorkerReply::Family(reply) => Ok(reply),
            WorkerReply::Failed { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::other("unexpected family worker response")),
        }
    }
}
impl crate::screen::core::host_backend::ScreenInputSink for WorkerInputSink {
    fn legacy_input_invalid(&self) -> bool {
        self.1.legacy_reconnect.load(Ordering::Acquire)
    }
    /// Input the host process's sink refused after its answer (the Wayland
    /// portal), which the process sent unasked (`INPUT_REFUSED`, review
    /// 2026-11-19); whoever takes it first reports it, this or the answer
    /// to the next `INPUT`.
    fn take_refusal(&self) -> Option<crate::screen::core::wire_control::ControlBlock> {
        self.1
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
    fn inject(
        &self,
        events: &[crate::screen::core::input_model::InputEvent],
        reference: u64,
        presented: bool,
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        backend::send_bound_input(&self.0, events, Some(reference), presented)
    }
    fn family_snapshot(&self) -> io::Result<Option<crate::remote_window::FamilySnapshot>> {
        match self.family(
            crate::screen::core::worker_family::FamilyCommand::Snapshot,
            None,
        )? {
            crate::screen::core::worker_family::FamilyReply::Snapshot(snapshot) => Ok(snapshot),
            _ => Err(io::Error::other("unexpected family snapshot response")),
        }
    }
    fn family_operation(
        &self,
        request: crate::screen::core::family_protocol::FamilyOperation,
        active: Arc<AtomicBool>,
    ) -> io::Result<crate::screen::core::family_protocol::FamilyOperationResult> {
        match self.family(
            crate::screen::core::worker_family::FamilyCommand::Operation(request),
            Some(active),
        )? {
            crate::screen::core::worker_family::FamilyReply::Operation(result) => Ok(result),
            _ => Err(io::Error::other("unexpected family operation response")),
        }
    }
    fn family_input(
        &self,
        request: crate::screen::core::family_protocol::FamilyInput,
        active: Arc<AtomicBool>,
    ) -> io::Result<crate::screen::core::family_protocol::FamilyInputResult> {
        match self.family(
            crate::screen::core::worker_family::FamilyCommand::Input(request),
            Some(active),
        )? {
            crate::screen::core::worker_family::FamilyReply::Input(result) => Ok(result),
            _ => Err(io::Error::other("unexpected family input response")),
        }
    }
    fn set_cursor_overlay(&self, on: bool) -> bool {
        use crate::screen::core::worker_capture_wire::{CaptureReply, CaptureRequest};
        let Ok(mut link) = self.0.lock() else {
            return false;
        };
        let request = WorkerRequest::CaptureControl(CaptureRequest::CursorOverlay(on));
        matches!(
            link.ask(&request, INPUT_TIMEOUT),
            Ok(WorkerReply::CaptureControl(CaptureReply::CursorOverlay(
                true
            )))
        )
    }
    fn cursor_feed(&self) -> Option<Arc<crate::screen::core::cursor_overlay::CursorFeed>> {
        Some(self.1.cursor.clone())
    }
    fn release(
        &self,
        events: &[crate::screen::core::input_model::InputEvent],
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        use crate::screen::core::wire_control::ControlBlock;
        // The picture request already has FRAME_TIMEOUT; retain this exact worker
        // until its admitted capture completes, then release its pressed inputs.
        let mut link = self
            .0
            .lock()
            .map_err(|_| ControlBlock::ControlUnavailable)?;
        match link.ask(&WorkerRequest::Release(events.to_vec()), INPUT_TIMEOUT) {
            Ok(WorkerReply::InputDone { blocked: None, .. }) => Ok(()),
            Ok(WorkerReply::InputDone {
                blocked: Some(blocked),
                ..
            }) => Err(blocked),
            _ => Err(ControlBlock::ControlUnavailable),
        }
    }
}

impl WorkerSource {
    pub fn open_family(
        program: &Path,
        env: &[(String, String)],
        open: crate::screen::core::worker_family::OpenFamily,
    ) -> Result<Self, SourceRefusal> {
        let mut link = WorkerLink::start(program, env)
            .map_err(|error| SourceRefusal::new(&format!("host_process: {error}")))?;
        match link.ask(&WorkerRequest::OpenFamily(open), OPEN_TIMEOUT) {
            Ok(WorkerReply::Opened { size }) => Ok(Self {
                broker: link.broker.clone(),
                facts: link.facts.clone(),
                link: Arc::new(Mutex::new(link)),
                size,
            }),
            Ok(WorkerReply::Refused { reason }) => Err(SourceRefusal::new(&reason)),
            Ok(_) => Err(SourceRefusal::new("unexpected OPEN_FAMILY response")),
            Err(error) => Err(SourceRefusal::new(&error.error.to_string())),
        }
    }
    /// Starts a host process with `program` (and `env` beside the daemon's
    /// environment) and opens `source` there.
    /// `control`: the session may control, so the process prepares input.
    pub fn open(
        program: &Path,
        env: &[(String, String)],
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
    ) -> Result<Self, SourceRefusal> {
        let refusal = |error: io::Error| SourceRefusal::new(&format!("host_process: {error}"));
        let link = WorkerLink::start(program, env).map_err(refusal)?;
        Self::opened(link, source, wanted, control)
    }

    fn opened(
        mut link: WorkerLink,
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
    ) -> Result<Self, SourceRefusal> {
        let request = WorkerRequest::Open(OpenRequest {
            source: source.clone(),
            wanted,
            control,
        });
        match link.ask(&request, OPEN_TIMEOUT) {
            Ok(WorkerReply::Opened { size }) => Ok(Self {
                broker: link.broker.clone(),
                facts: link.facts.clone(),
                link: Arc::new(Mutex::new(link)),
                size,
            }),
            Ok(WorkerReply::Refused { reason }) => Err(SourceRefusal::new(&reason)),
            Ok(_) => Err(SourceRefusal::new(
                "host_process: unexpected answer to OPEN",
            )),
            Err(failure) => Err(SourceRefusal::new(&format!(
                "host_process: {}",
                failure.error
            ))),
        }
    }

    /// The link, for the input path of the backend.
    pub(super) fn shared_link(&self) -> SharedLink {
        self.link.clone()
    }

    pub(super) fn link(&self) -> MutexGuard<'_, WorkerLink> {
        self.link.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn unexpected(&mut self, what: &str) -> StageError {
        self.link().kill();
        StageError::new(
            ScreenErrorCode::CaptureFailed,
            io::Error::other(format!(
                "unexpected answer of the screen host process to {what}"
            )),
        )
    }
}

impl ScreenSource for WorkerSource {
    fn capture_grant(&self) -> Option<crate::screen::core::capture_admission::CaptureGrant> {
        self.broker.current()
    }
    fn family_layout_revision(&self) -> Option<u64> {
        let revision = self.facts.family_layout_revision.load(Ordering::Acquire);
        (revision != 0).then_some(revision)
    }
    fn frame_limits(&self) -> FrameLimits {
        *self
            .facts
            .limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
    fn input_sink(&self) -> Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {
        Some(Arc::new(WorkerInputSink(
            self.link.clone(),
            self.facts.clone(),
        )))
    }
    fn reference_generation(&self) -> u64 {
        self.facts.generation.load(Ordering::Acquire)
    }
    fn supports_frame_regions(&self) -> bool {
        self.facts.regions.load(Ordering::Acquire)
    }
    fn size(&self) -> FrameSize {
        self.size
    }

    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        match self
            .link()
            .ask(&WorkerRequest::SetCaptureActive(active), FRAME_TIMEOUT)
        {
            Ok(WorkerReply::CaptureActive(value)) if value == active => Ok(()),
            Ok(WorkerReply::Failed { message, .. }) => Err(io::Error::other(message)),
            Ok(_) => Err(io::Error::other(
                "unexpected host capture activity acknowledgement",
            )),
            Err(failure) => Err(failure.error),
        }
    }

    /// One raw picture (still images); video goes through `host_stage`.
    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        self.capture_snapshot(frame_id, None)
    }
    fn capture_snapshot(
        &mut self,
        frame_id: u64,
        frame_contract: Option<crate::screen::core::frame_budget::FrameContract>,
    ) -> io::Result<SourceFrame> {
        let answer = self.link().ask(
            &WorkerRequest::Capture {
                frame_id,
                frame_contract,
            },
            FRAME_TIMEOUT,
        );
        match answer {
            Ok(WorkerReply::Raw(image)) => {
                if frame_contract.is_none() {
                    self.size = image.size();
                }
                Ok(SourceFrame::Frame(image))
            }
            Ok(WorkerReply::Frame(StageFrame::Paused(state))) => Ok(SourceFrame::Paused(state)),
            Ok(WorkerReply::Failed { message, .. }) => Err(io::Error::other(message)),
            Ok(_) => Err(self.unexpected("CAPTURE").error),
            Err(failure) => Err(failure.error),
        }
    }

    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for WorkerSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        // A process that paces itself streams (plan „Bildkette“ B-27).
        if self.streams() {
            return self.produce_streamed(request);
        }
        let answer = self
            .link()
            .ask(&WorkerRequest::Produce(request.clone()), FRAME_TIMEOUT)?;
        match answer {
            WorkerReply::Frame(frame) => {
                match &frame {
                    StageFrame::Encoded { source_size, .. }
                    | StageFrame::Unchanged { source_size } => self.size = *source_size,
                    StageFrame::Paused(_) => {}
                }
                Ok(frame)
            }
            WorkerReply::Failed { code, message } => {
                Err(StageError::new(code, io::Error::other(message)))
            }
            _ => Err(self.unexpected("PRODUCE")),
        }
    }

    /// Paces its own pictures: the daemon grants credit when it frees,
    /// not per tick of its session loop (BK3b).
    fn self_paced(&self) -> bool {
        self.streams()
    }

    fn set_stream_hint(&mut self, hint: crate::screen::core::host_stage::StreamHint) {
        self.keep_stream_hint(hint);
    }

    fn take_stream_report(&mut self) -> Option<crate::screen::core::host_stage::StreamReport> {
        self.last_stream_report()
    }

    fn drain_chain(&mut self, meter: &mut crate::screen::core::chain_meter::ChainMeter) {
        self.drain_stream_chain(meter);
    }

    /// The lowest bit rate the host process reported (`ENCODER_FLOOR`,
    /// review 2026-11-19 X3) against the viewer's ceiling, once per pair.
    fn floor_notice(&mut self, ceiling_bps: Option<u64>) -> Option<String> {
        let floor = self.facts.floor.load(Ordering::Acquire);
        let mut noticed = self
            .facts
            .floor_noticed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(text) = crate::screen::core::host_stage::floor_above_ceiling(floor, ceiling_bps)
        else {
            *noticed = None;
            return None;
        };
        let pair = (floor, ceiling_bps.unwrap_or(0));
        if *noticed == Some(pair) {
            return None;
        }
        *noticed = Some(pair);
        Some(text)
    }

    /// B-36: only the picture stream carries the wish (`STREAM`); `PRODUCE`
    /// keeps the reset with a new encoder.
    fn continues_references(&self) -> bool {
        self.streams()
    }

    fn recover_references(&mut self, last_good: u64) {
        self.keep_recovery(last_good);
    }
}
