//! Pipeline construction and immutable session-memory binding.
use super::*;

impl Pipeline {
    pub fn new(
        backend: Arc<dyn ScreenHostBackend>,
        source: Box<dyn ScreenSource>,
        choice: CodecChoice,
        connection: &Connection,
        max_fps: u32,
        display: Option<FrameSize>,
        inject_resets: Arc<AtomicU32>,
    ) -> Self {
        let source_size = source.size();
        let source_generation = source.reference_generation();
        let viewer_fps = max_fps;
        let quality_policy = QualityPolicy::new(choice, max_fps);
        let max_fps = quality_policy.max_fps();
        let (path, _) = path_of(connection);
        let display = display.unwrap_or(FrameSize::MAX);
        let chain = super::super::chain_meter::SharedChain::default();
        let mut pipeline = Self {
            work: Arc::new(work_control::WorkControl::default()),
            backend,
            frame_contract: None,
            negotiated_contract: None,
            memory: None,
            admission: None,
            sender: super::super::sender_budget::SenderBudget::for_peer(
                connection.remote_id().to_string(),
            ),
            route_epoch: None,
            source_size,
            source_generation,
            presentation: ReferencePresentation::new(),
            capture_active: true,
            capture_applied: Some(true),
            capture_requested: true,
            capture_ready: None,
            capture_transition: None,
            visible_rects: None,
            stage: Some(Stage {
                memory: None,
                source,
                encoder: None,
                encoder_book: None,
                refused: Vec::new(),
                chain: chain.clone(),
                visible: None,
            }),
            choice,
            quality_policy,
            fallback_notice: None,
            tile_limit: (choice.codec == CodecKind::Tiles).then_some(choice.max_size),
            kind: StreamKind::Video,
            gop: None,
            next_gop_id: 1,
            next_frame_id: 1,
            pacer: Pacer::default(),
            rate: RateController::new(path, now_us()),
            clock: FrameClock::new(max_fps),
            codec_fps: 0,
            self_paced: false,
            chain_ready: false,
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
            pairing: pairing::PairingState::new(choice, viewer_fps),
            confirmed: ConfirmedReference::default(),
            continue_from: None,
            kept_encoder: false,
            key_gop: 0,
            chain,
            stale_resets: 0,
        };
        pipeline.quality.size = pipeline.fitted_size();
        pipeline.resize();
        // User 2026-11-25 („ich WILL NICHT DASS DU DROSSELST“): no estimate
        // lowers the rate and no guard drops a picture; the „Übertragung“
        // decides about late pictures (`delivery.rs`).
        pipeline.rate.set_unthrottled(true);
        pipeline.pacer.set_unthrottled(true);
        pipeline
    }

    /// The viewer's „Übertragung“ (`delivery.rs`).
    pub fn set_delivery(&mut self, delivery: super::super::delivery::Delivery) {
        if self.pacer.delivery() != delivery {
            self.note_restart(&format!("delivery {}", delivery.key()));
        }
        self.pacer.set_delivery(delivery);
    }

    pub fn delivery(&self) -> super::super::delivery::Delivery {
        self.pacer.delivery()
    }

    /// The line's available rate a measurement found (`bandwidth_probe`):
    /// the encoder gets it from the next picture on.
    pub fn set_measured_bps(&mut self, bps: u64) {
        self.rate.set_measured(bps);
        self.note_restart(&format!(
            "the line was measured at {} kbit/s: the encoder gets it",
            bps / 1000
        ));
    }

    pub fn measured_bps(&self) -> Option<u64> {
        self.rate.measured_bps()
    }

    /// What the bandwidth probe watches of the line at `now_us`: the stale
    /// resets so far and when the oldest unacknowledged picture left.
    pub fn line_facts(&self, now_us: u64) -> super::super::bandwidth_probe::LineFacts {
        super::super::bandwidth_probe::LineFacts {
            stale_resets: self.stale_resets,
            oldest_unacked_sent_us: self
                .pacer
                .oldest_age_us(now_us)
                .map(|age| now_us.saturating_sub(age)),
        }
    }

    pub fn hold_admission(
        &mut self,
        admission: Arc<super::super::capture_admission::SessionAdmission>,
    ) {
        self.admission = Some(admission);
    }

    pub fn hold_memory(&mut self, memory: std::sync::Arc<super::super::frame_memory::MemoryLease>) {
        if let Some(stage) = &mut self.stage {
            stage.memory = Some(memory.clone());
        }
        self.memory = Some(memory);
    }
    pub fn set_frame_contract(
        &mut self,
        contract: Option<super::super::frame_budget::FrameContract>,
    ) -> io::Result<()> {
        if let Some(contract) = contract {
            contract.validate()?;
            // The viewer's display from its hello within the contract
            // (Gemini 3/5: the contract's largest size replaced it, so the
            // first picture had the source size and the first `SET_MODE`
            // with the window size opened the encoder again).
            let max = contract.limits.max_size;
            self.display = FrameSize::new(
                self.display.width.min(max.width),
                self.display.height.min(max.height),
            );
            self.bounds = self.display;
        }
        self.negotiated_contract = contract;
        self.frame_contract = contract.map(|mut c| {
            c.hybrid_regions &= self
                .stage
                .as_ref()
                .is_some_and(|stage| stage.source.supports_frame_regions());
            c
        });
        self.quality.size = self.fitted_size();
        self.note_restart("frame contract set: new encoder");
        self.invalidate_reference();
        Ok(())
    }
}
