//! Pipeline construction and immutable session-memory binding.
use super::*;

impl Pipeline {
    pub(crate) fn new(
        backend: Arc<dyn ScreenHostBackend>,
        source: Box<dyn ScreenSource>,
        choice: CodecChoice,
        connection: &Connection,
        max_fps: u32,
        display: Option<FrameSize>,
        inject_resets: Arc<AtomicU32>,
    ) -> Self {
        let source_size=source.size();
        let source_generation=source.reference_generation();
        let quality_policy = QualityPolicy::new(choice, max_fps);
        let max_fps = quality_policy.max_fps();
        let (path, _) = path_of(connection);
        let display = display.unwrap_or(FrameSize::MAX);
        let mut pipeline = Self {
            work:Arc::new(work_control::WorkControl::default()),
            backend,
            frame_contract: None,
            negotiated_contract: None, memory:None, admission:None,
            sender: super::super::sender_budget::SenderBudget::for_peer(connection.remote_id().to_string()),
            route_epoch: None,
            source_size,source_generation,presentation:ReferencePresentation::new(),
            capture_active: true, capture_applied:Some(true), capture_requested:true,capture_ready:None,capture_transition:None,visible_rects: None,
            stage: Some(Stage {
                memory:None,
                source,
                encoder: None,
                encoder_book: None,
            }),
            choice,
            quality_policy,
            fallback_notice: None,
            tile_limit: (choice.codec==CodecKind::Tiles).then_some(choice.max_size),
            kind: StreamKind::Video,
            gop: None,
            next_gop_id: 1,
            next_frame_id: 1,
            pacer: Pacer::default(),
            rate: RateController::new(path, now_us()),
            clock: FrameClock::new(max_fps),codec_fps:0,
            quality: Quality {
                fps: max_fps,
                size: FrameSize::MAX,
            },
            max_fps,
            display,
            bounds: display,
            preview_interval_us: PREVIEW_INTERVAL_US,
            preview_in_flight: None,
            preview_sent_us: 0,
            totals: Totals::default(),
            second_bytes: 0,
            second_frames: 0,
            inject_resets,
            encoder_epoch: 0,
            running: None,
        };
        pipeline.quality.size = pipeline.fitted_size();
        pipeline.resize();
        pipeline
    }

    pub(crate) fn hold_admission(&mut self,admission:Arc<super::super::capture_admission::SessionAdmission>) {self.admission=Some(admission);}

    pub(crate) fn hold_memory(&mut self,memory:std::sync::Arc<super::super::frame_memory::MemoryLease>) {
        if let Some(stage)=&mut self.stage {stage.memory=Some(memory.clone());}
        self.memory=Some(memory);
    }
    pub(crate) fn set_frame_contract(&mut self, contract: Option<super::super::frame_budget::FrameContract>) -> io::Result<()> {
        if let Some(contract) = contract { contract.validate()?; self.bounds = contract.limits.max_size; self.display = self.bounds; }
        self.negotiated_contract = contract;
        self.frame_contract = contract.map(|mut c| {c.hybrid_regions &= self.stage.as_ref().is_some_and(|stage|stage.source.supports_frame_regions());c}); self.quality.size = self.fitted_size(); self.invalidate_reference(); Ok(())
    }

}
