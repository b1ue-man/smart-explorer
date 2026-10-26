//! One owned picture operation, coalesced control state, bounded receipt metadata.
use super::*;
use crate::screen::core::wire_control::{FrameAck, HostStats};
use std::collections::VecDeque;
use std::future::Future;

pub(crate) enum WorkCompletion {
    Tick(Result<Produced, PipelineError>),
    Snapshot(Result<(), PipelineError>),
}
enum Receipt {
    Ack(FrameAck),
    Presented(u64, u64),
}
#[derive(Default)]
struct Pending {
    source: Option<Box<dyn ScreenSource>>,
    mode: Option<QualityMode>,
    visible: Option<Option<Vec<crate::screen::core::visibility::VisibleRect>>>,
    active: Option<bool>,
    invalidate: bool,
    keyframe: bool,
    second: bool,
    snapshot: Option<Option<FrameSize>>,
    receipts: VecDeque<Receipt>,
}
struct Facts {
    next: u64,
    floor: u64,
    reference: u64,
    source_generation: u64,
    presentation: bool,
    ready: bool,
    keyframe: bool,
    stream: Option<StreamInfo>,
    totals: Totals,
    stats: HostStats,
}
pub(crate) struct PipelineActor {
    pipeline: Option<Pipeline>,
    connection: Connection,
    work: Arc<work_control::WorkControl>,
    job: Option<tokio::task::JoinHandle<(Pipeline, WorkCompletion)>>,
    job_epoch: u64,
    pending: Pending,
    completion: Option<WorkCompletion>,
    capture_error: Option<io::Error>,
    fallback_notice: Option<String>,
    facts: Facts,
    blocked: bool,
    stopped: bool,
    known_ack: Option<u64>,
    input_sink: Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>>,
    route: Option<(
        u64,
        u64,
        Option<crate::share::route_state::RouteClass>,
        bool,
    )>,
    mode: QualityMode,
    admission: Option<Arc<crate::screen::core::capture_admission::SessionAdmission>>,
    accepted: Option<crate::screen::core::frame_budget::FrameContract>,
    capture_codec: crate::screen::core::host_codec_adapters::HostCodecPlan,
}
impl PipelineActor {
    pub(crate) fn new(mut pipeline: Pipeline, connection: Connection) -> Self {
        let stats = pipeline.second(&connection);
        let admission = pipeline.admission.clone();
        let accepted = pipeline.negotiated_contract;
        let capture_codec = if pipeline.choice.codec == CodecKind::Tiles {
            crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles
        } else if pipeline.backend.platform()
            == crate::screen::core::wire_hello::HostPlatform::Windows
        {
            crate::screen::core::host_codec_adapters::HostCodecPlan::MediaFoundation {
                output_buffer_bytes: 64 * 1024 * 1024,
            }
        } else {
            crate::screen::core::host_codec_adapters::HostCodecPlan::OpenH264
        };
        let work = pipeline.work.clone();
        let input_sink = pipeline
            .stage
            .as_ref()
            .and_then(|stage| stage.source.input_sink());
        let mode = pipeline.quality_policy.mode;
        let facts = Facts {
            next: pipeline.next_frame_id(),
            floor: pipeline.reference_floor(),
            reference: work.reference(),
            source_generation: pipeline.input_reference_generation(),
            presentation: pipeline.presentation_supported(),
            ready: pipeline.input_reference_ready(),
            keyframe: pipeline.pacer.keyframe_due(),
            stream: pipeline.stream_info(),
            totals: pipeline.totals(),
            stats,
        };
        Self {
            pipeline: Some(pipeline),
            connection,
            work,
            job: None,
            job_epoch: 0,
            pending: Pending::default(),
            completion: None,
            capture_error: None,
            fallback_notice: None,
            facts,
            blocked: false,
            stopped: false,
            known_ack: None,
            input_sink,
            route: None,
            mode,
            admission,
            accepted,
            capture_codec,
        }
    }
    fn refresh(&mut self) {
        if let Some(notice) = self
            .pipeline
            .as_mut()
            .and_then(|p| p.fallback_notice.take())
        {
            self.fallback_notice = Some(notice);
        }
        if let Some(p) = self.pipeline.as_ref() {
            self.facts.next = p.next_frame_id();
            self.facts.floor = p.reference_floor();
            self.facts.reference = self.work.reference();
            self.facts.source_generation = p.input_reference_generation();
            self.facts.presentation = p.presentation_supported();
            self.facts.ready = p.input_reference_ready();
            self.facts.keyframe = p.pacer.keyframe_due();
            self.facts.stream = p.stream_info();
            self.facts.totals = p.totals();
        }
    }
    fn interrupt(&mut self) {
        self.work.interrupt();
        self.blocked = true;
        self.pending.snapshot = None;
        self.pending.invalidate = true;
    }
    fn receipt(&mut self, receipt: Receipt) {
        // Same bound as HostSession's control inbox; only acknowledgements, never pixels.
        if self.pending.receipts.len() == 64 {
            self.pending.receipts.pop_front();
        }
        self.pending.receipts.push_back(receipt);
    }
    pub(crate) fn poll(&mut self) {
        let route = crate::share::route_monitor::snapshot(&self.connection).map(|route| {
            (
                route.generation,
                route.pause_epoch,
                route.class,
                route.application_allowed,
            )
        });
        if route != self.route {
            self.route = route;
            self.interrupt();
            self.pending.invalidate = true;
        }
        if let Some(mut job) = self.job.take() {
            let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
            match std::pin::Pin::new(&mut job).poll(&mut cx) {
                std::task::Poll::Pending => self.job = Some(job),
                std::task::Poll::Ready(Ok((pipeline, mut completion))) => {
                    if !self.work.current(self.job_epoch) {
                        completion = match completion {
                            WorkCompletion::Tick(_) => WorkCompletion::Tick(Ok(Produced::Reset)),
                            WorkCompletion::Snapshot(_) => WorkCompletion::Snapshot(Err(
                                PipelineError::capture(io::Error::from(io::ErrorKind::Interrupted)),
                            )),
                        };
                    }
                    self.pipeline = Some(pipeline);
                    let cancelled = matches!(&completion,WorkCompletion::Snapshot(Err(error)) if error.error.kind()==io::ErrorKind::Interrupted);
                    if !cancelled {
                        self.completion = Some(completion);
                    }
                }
                std::task::Poll::Ready(Err(error)) => {
                    self.stopped = true;
                    self.blocked = true;
                    self.completion = Some(WorkCompletion::Tick(Err(PipelineError::capture(
                        io::Error::other(error),
                    ))));
                }
            }
        }
        self.apply();
    }
    fn apply(&mut self) {
        let Some(mut p) = self.pipeline.take() else {
            return;
        };
        if let Some(source) = self.pending.source.take() {
            p.set_source(source);
        }
        if let Some(mode) = self.pending.mode.take() {
            p.set_quality_mode(mode);
        }
        if let Some(rects) = self.pending.visible.take() {
            p.set_visible_rects(rects);
        }
        if let Some(active) = self.pending.active.take() {
            if let Err(error) = p.begin_capture_active(active) {
                self.capture_error = Some(error);
            }
        }
        if std::mem::take(&mut self.pending.invalidate) {
            p.invalidate_reference();
        }
        if std::mem::take(&mut self.pending.keyframe) {
            p.request_keyframe();
        }
        if std::mem::take(&mut self.pending.second) {
            self.facts.stats = p.second(&self.connection);
        }
        while let Some(receipt) = self.pending.receipts.pop_front() {
            match receipt {
                Receipt::Ack(ack) => {
                    if p.on_ack(ack) {
                        self.known_ack = Some(ack.frame_id);
                    }
                }
                Receipt::Presented(gop, frame) => {
                    p.on_presented(gop, frame);
                }
            }
        }
        self.pipeline = Some(p);
        self.blocked = self.stopped;
        self.refresh();
    }
    pub(crate) fn take_completion(&mut self) -> Option<WorkCompletion> {
        self.poll();
        self.completion.take()
    }
    pub(crate) fn take_fallback_notice(&mut self) -> Option<String> {
        self.poll();
        self.fallback_notice.take()
    }
    pub(crate) fn begin_tick(&mut self) -> Result<(), PipelineError> {
        self.poll();
        if self.stopped || self.job.is_some() || self.completion.is_some() {
            return Ok(());
        }
        let Some(p) = self.pipeline.as_ref() else {
            return Ok(());
        };
        if p.capture_transition.is_some() || !p.capture_active {
            return Ok(());
        }
        self.start(None)
    }
    fn start(&mut self, requested: Option<Option<FrameSize>>) -> Result<(), PipelineError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|e| PipelineError::capture(io::Error::other(e)))?;
        let Some(mut p) = self.pipeline.take() else {
            return Ok(());
        };
        let snapshot = requested.or_else(|| self.pending.snapshot.take());
        let connection = self.connection.clone();
        let epoch = self.work.epoch();
        self.job_epoch = epoch;
        self.job = Some(runtime.spawn(async move {
            let result = match snapshot {
                Some(max) => {
                    WorkCompletion::Snapshot(p.snapshot_epoch(&connection, max, epoch).await)
                }
                None => WorkCompletion::Tick(p.tick_epoch(&connection, epoch).await),
            };
            (p, result)
        }));
        Ok(())
    }
    pub(crate) fn snapshot(&mut self, max: Option<FrameSize>) -> Result<(), PipelineError> {
        self.poll();
        if self.stopped || self.blocked || self.pending.snapshot.is_some() {
            return Err(PipelineError::capture(io::Error::new(
                io::ErrorKind::WouldBlock,
                "snapshot unavailable during pending picture work",
            )));
        }
        if self
            .pipeline
            .as_ref()
            .is_some_and(|p| p.capture_transition.is_some() || !p.capture_active)
        {
            return Err(PipelineError::capture(io::Error::new(
                io::ErrorKind::WouldBlock,
                "capture transition pending",
            )));
        }
        if self.job.is_some() || self.completion.is_some() {
            self.pending.snapshot = Some(max);
            Ok(())
        } else {
            self.start(Some(max))
        }
    }
    pub(crate) fn begin_capture_active(&mut self, active: bool) -> io::Result<()> {
        if self.stopped {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "pipeline stopped",
            ));
        }
        self.interrupt();
        self.pending.active = Some(active);
        self.poll();
        Ok(())
    }
    pub(crate) fn poll_capture_active(&mut self) -> Option<io::Result<()>> {
        self.poll();
        if let Some(error) = self.capture_error.take() {
            return Some(Err(error));
        }
        let result = self.pipeline.as_mut()?.poll_capture_active();
        self.refresh();
        result
    }
    pub(crate) fn capture_context(
        &self,
    ) -> io::Result<(
        Arc<crate::screen::core::capture_admission::SessionAdmission>,
        crate::screen::core::host_codec_adapters::HostOutputPlan,
    )> {
        use crate::screen::core::host_codec_adapters::{HostContract, HostOutputPlan};
        let admission = self
            .admission
            .clone()
            .ok_or_else(|| io::Error::other("source switch has no session admission"))?;
        let limits = match self.accepted {
            Some(contract) => contract.limits,
            None => {
                crate::screen::core::frame_budget::legacy_limits(admission.working_bytes(), None)?
            }
        };
        let output = HostOutputPlan::new(
            self.accepted
                .map_or(HostContract::Legacy, HostContract::Versioned),
            limits.fit(limits.max_size),
            self.capture_codec,
        )?;
        let output = if self.accepted.is_none() {
            output.with_legacy_limits(limits)?
        } else {
            output
        };
        Ok((admission, output))
    }
    pub(crate) fn set_source(&mut self, source: Box<dyn ScreenSource>) {
        self.interrupt();
        self.input_sink = source.input_sink();
        self.pending.source = Some(source);
        self.poll();
    }
    pub(crate) fn set_mode(
        &mut self,
        kind: StreamKind,
        size: Option<FrameSize>,
        fps: Option<u16>,
        sharpness: bool,
        lossless: bool,
    ) {
        let mode = self.mode.update(kind, size, fps, sharpness, lossless);
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.interrupt();
        self.pending.mode = Some(mode);
        self.poll();
    }
    pub(crate) fn set_visible_rects(
        &mut self,
        rects: Option<Vec<crate::screen::core::visibility::VisibleRect>>,
    ) {
        self.interrupt();
        self.pending.visible = Some(rects);
        self.poll();
    }
    pub(crate) fn invalidate_reference(&mut self) {
        self.interrupt();
        self.pending.invalidate = true;
        self.poll();
    }
    pub(crate) fn request_keyframe(&mut self) {
        if self.facts.keyframe || self.facts.reference != self.work.reference() {
            return;
        }
        self.interrupt();
        self.pending.keyframe = true;
        self.poll();
    }
    pub(crate) fn on_ack(&mut self, ack: FrameAck) -> bool {
        self.poll();
        if let Some(p) = self.pipeline.as_mut() {
            let known = p.on_ack(ack);
            if known {
                self.known_ack = Some(ack.frame_id);
            }
            self.refresh();
            known
        } else {
            self.receipt(Receipt::Ack(ack));
            false
        }
    }
    pub(crate) fn on_presented(&mut self, gop: u64, frame: u64) -> bool {
        self.poll();
        if let Some(p) = self.pipeline.as_mut() {
            let accepted = p.on_presented(gop, frame);
            self.refresh();
            accepted
        } else {
            self.receipt(Receipt::Presented(gop, frame));
            false
        }
    }
    pub(crate) fn second(&mut self, _connection: &Connection) -> HostStats {
        self.pending.second = true;
        self.poll();
        self.facts.stats
    }
    pub(crate) fn next_frame_id(&self) -> u64 {
        self.facts.next
    }
    pub(crate) fn input_reference_generation(&self) -> u64 {
        self.facts.source_generation
    }
    pub(crate) fn reference_floor(&self) -> u64 {
        self.facts.floor
    }
    pub(crate) fn presentation_supported(&self) -> bool {
        self.facts.presentation
    }
    pub(crate) fn input_reference_ready(&self) -> bool {
        !self.blocked && self.facts.reference == self.work.reference() && self.facts.ready
    }
    pub(crate) fn known_ack(&self) -> Option<u64> {
        self.known_ack
    }
    pub(crate) fn input_sink(
        &self,
    ) -> Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {
        self.input_sink.clone()
    }
    pub(crate) fn stream_info(&self) -> Option<StreamInfo> {
        self.facts.stream.clone()
    }
    pub(crate) fn totals(&self) -> Totals {
        self.facts.totals
    }
    pub(crate) fn cancel(&mut self) {
        self.interrupt();
        self.stopped = true;
        self.pending = Pending::default();
        if let Some(p) = self.pipeline.as_mut() {
            p.reset_stream();
        }
    }
}
fn dispose(pipeline: Pipeline) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let _ = runtime.spawn_blocking(move || drop(pipeline));
    } else {
        drop(pipeline);
    }
}
impl Drop for PipelineActor {
    fn drop(&mut self) {
        self.work.interrupt();
        if let Some(pipeline) = self.pipeline.take() {
            dispose(pipeline);
        }
        if let Some(job) = self.job.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let _ = runtime.spawn(async move {
                    if let Ok((pipeline, _)) = job.await {
                        dispose(pipeline);
                    }
                });
            }
        }
    }
}
