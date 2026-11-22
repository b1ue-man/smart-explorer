//! Viewer control frames, input authorization, control release and the
//! pointer overlay (P6) for one session.
use super::*;
use crate::screen::core::cursor_overlay::{CursorFeed, CursorPosition, CursorSeen, CursorUpdate};
use crate::screen::core::wire_control::{ControlBlock, InputHeld};

/// The pointer overlay of one session (plan 2026-11-14 P6).
#[derive(Default)]
pub(super) struct CursorLink {
    /// The viewer asked for the overlay; every new source is asked again.
    wanted: bool,
    /// Where the pointer of the session's source arrives, and what of it
    /// this session has sent on.
    feed: Option<(std::sync::Arc<CursorFeed>, CursorSeen)>,
    /// The last wish on its way to a source; the next one waits for it, so
    /// a source hears the wishes in their order.
    switching: Option<tokio::task::JoinHandle<()>>,
}

impl CursorLink {
    /// Resolves when the source's pointer changed; never without a feed.
    pub(super) async fn changed(&self) {
        match &self.feed {
            Some((feed, _)) => feed.changed().await,
            None => std::future::pending().await,
        }
    }
}

impl Session<'_> {
    pub(super) async fn on_frame(&mut self, frame: ViewerFrame) -> io::Result<()> {
        self.pipeline.poll();
        self.poll_input_release();
        self.settle_input().await?;
        self.refresh_input_reference();
        self.end_hold().await?;
        self.report_refusal().await?;
        match frame {
            ViewerFrame::ListFamilies => {
                if !self.family.supported {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "family_contract_not_negotiated",
                    ));
                }
                self.push_metadata(crate::screen::core::host_metadata::Request::Families)
                    .await
            }
            ViewerFrame::FamilySnapshot => self.family_snapshot(true).await,
            ViewerFrame::FamilyOperation(request) => self.family_operation(request).await,
            ViewerFrame::FamilyInput(request) => self.family_input(request).await,
            ViewerFrame::Presented { gop_id, frame_id } => {
                if !self.pipeline.presentation_supported() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "presentation was not negotiated",
                    ));
                }
                self.pipeline.note_presented(frame_id);
                if self.visible
                    && !self.visibility_ack_pending
                    && self.pipeline.on_presented(gop_id, frame_id)
                {
                    self.input_floor = None;
                }
                Ok(())
            }
            ViewerFrame::FrameAck(ack) | ViewerFrame::PreviewAck(ack) => {
                let known = self.pipeline.on_ack(ack);
                if !self.pipeline.presentation_supported()
                    && known
                    && self.input_floor.is_some_and(|floor| ack.frame_id >= floor)
                    && ack.frame_id < self.pipeline.next_frame_id()
                {
                    self.input_floor = None;
                }
                Ok(())
            }
            ViewerFrame::Visibility(report) => self.visibility(report).await,
            ViewerFrame::KeyframeRequest(request) => {
                self.pipeline
                    .request_keyframe(request.gop_id, request.reason);
                Ok(())
            }
            ViewerFrame::Ping(ping) => {
                let received = now_us();
                let pong = Pong::answer(&ping, received, now_us());
                self.frame(HostFrame::Pong(pong)).await
            }
            ViewerFrame::SetMode(set) => self.set_mode(set).await,
            ViewerFrame::ListWindows => {
                self.push_metadata(crate::screen::core::host_metadata::Request::Windows)
                    .await
            }
            ViewerFrame::IconRequest(request) => {
                if !self.icons_supported {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "icon contract was not negotiated",
                    ));
                }
                request.validate()?;
                self.push_metadata(crate::screen::core::host_metadata::Request::Icon(request))
                    .await
            }
            ViewerFrame::Snapshot(request) => {
                // The source is the session's own (hello or `SET_MODE`); the
                // frame's `source` stays on the wire for later viewers and is
                // not used here (SCR-M4, `se screen snapshot` names its source
                // in the session request).
                match self.pipeline.snapshot(request.max_size) {
                    Ok(()) => Ok(()),
                    Err(failure) => self.error(failure.code, failure.error.to_string()).await,
                }
            }
            ViewerFrame::Input(events) => self.input(&events).await,
            ViewerFrame::ControlRequest(request) => self.control(request.action).await,
            ViewerFrame::CursorOverlay(on) => self.cursor_overlay(on).await,
            // „Höchste Datenrate“ of the viewer (B-09, `rate_ceiling_v1`).
            ViewerFrame::RateCeiling(ceiling) => {
                self.pipeline.set_rate_ceiling(ceiling);
                // Held control frames follow the new ceiling at once, also
                // while a picture operation holds the pipeline (round 7 P7).
                self.outbox.follow_ceiling(ceiling);
                // The sound's rate follows it (Codex 7/7).
                if let Some(link) = self.context.backend.audio() {
                    link.set_ceiling(self.connection, ceiling);
                }
                self.sound_notice(Some(ceiling), None).await
            }
            // New decoders after „Codec“/„Dekodierer“ (`set_decoders_v1`):
            // a changed pairing starts a group in the new codec.
            ViewerFrame::SetDecoders(set) => {
                self.pipeline.set_decoders(set);
                Ok(())
            }
            // „Monitor“/„Auflösung“ in the session (P9, `set_display.rs`).
            ViewerFrame::SetDisplay(choice) => self.set_display(choice).await,
            // Station times on/off or one input probe (BK0, `frame_timing_v1`).
            ViewerFrame::FrameTiming(request) => {
                let mut meter = crate::screen::core::chain_meter::lock(self.pipeline.chain_meter());
                match request.probe_us {
                    Some(sent_us) => meter.on_input_probe(sent_us, now_us()),
                    None => meter.request_throttled(request.on),
                }
                Ok(())
            }
            // The host's sound on or off (P9); without an attached capture
            // link (no `audio` in `HELLO_OK`) nothing happens.
            ViewerFrame::Audio(on) => {
                if let Some(link) = self.context.backend.audio() {
                    link.set_wanted(self.connection, on);
                    return self.sound_notice(None, Some(on)).await;
                }
                Ok(())
            }
            // What the viewer's jitter buffer saw (P9): into the session's
            // view for `se screen sessions` and the host's tab.
            ViewerFrame::AudioReport(report) => {
                self.handle.update(|view| view.audio = Some(report));
                Ok(())
            }
        }
    }

    /// `CURSOR_REQUEST` (P6): when the source can separate the pointer it
    /// leaves the picture and travels as `CURSOR_SHAPE`/`CURSOR_POSITION`
    /// frames from now on; otherwise it stays in the picture and only the
    /// one "nothing to draw" position below is ever sent.
    pub(super) async fn cursor_overlay(&mut self, on: bool) -> io::Result<()> {
        super::visibility::note_viewer(&format!(
            "the viewer asked for the pointer {} (before: {})",
            if on {
                "beside the picture"
            } else {
                "in the picture"
            },
            if self.cursor.wanted { "beside" } else { "in" }
        ));
        self.cursor.wanted = on;
        self.apply_cursor().await
    }

    /// The session has a new source (`SET_MODE`), which starts with the
    /// pointer in its picture: a viewer that asked for the overlay gets it
    /// from this source too. A viewer that never asked hears nothing.
    pub(super) async fn cursor_source_changed(&mut self) -> io::Result<()> {
        if !self.cursor.wanted {
            return Ok(());
        }
        self.apply_cursor().await
    }

    /// Hands the viewer's wish to the session's source. The viewer first
    /// hears that there is nothing to draw: what it still holds belongs to
    /// the state before (the other source, the overlay that was on). The
    /// source answers only after its running picture operation (a capture
    /// that starts may wait for the host's user) and may switch later than
    /// it answers, so the wish travels beside this loop, which keeps hearing
    /// the viewer, and the feed is read whatever the answer is; a source that
    /// cannot separate the pointer never fills it. A window family keeps its
    /// pointers in the picture.
    async fn apply_cursor(&mut self) -> io::Result<()> {
        let on = self.cursor.wanted && self.family.selected.is_none();
        self.cursor.feed = None;
        self.frame(HostFrame::CursorPosition(CursorPosition::hidden()))
            .await?;
        let Some(sink) = self.pipeline.input_sink() else {
            return Ok(());
        };
        let feed = sink.cursor_feed();
        if let Some(feed) = &feed {
            feed.reset();
        }
        let before = self.cursor.switching.take();
        self.cursor.switching = Some(tokio::spawn(async move {
            if let Some(before) = before {
                let _ = before.await;
            }
            // A failed switch leaves the pointer in the picture and the feed empty.
            let _ = tokio::task::spawn_blocking(move || sink.set_cursor_overlay(on)).await;
        }));
        self.cursor.feed = feed
            .filter(|_| on)
            .map(|feed| (feed, CursorSeen::default()));
        Ok(())
    }

    /// Sends on what the source's pointer changed (P6): the shape when it
    /// changed, the newest position.
    pub(super) async fn poll_cursor(&mut self) -> io::Result<()> {
        let updates = match self.cursor.feed.as_mut() {
            Some((feed, seen)) => feed.poll(seen),
            None => return Ok(()),
        };
        for update in updates {
            let frame = match update {
                CursorUpdate::Shape(shape) => HostFrame::CursorShape(shape),
                CursorUpdate::Position(position) => HostFrame::CursorPosition(position),
            };
            self.frame(frame).await?;
        }
        Ok(())
    }

    /// One batch of the viewer (Codex 7/10): while an injection runs, it
    /// waits behind it in order (`InputHealth::backlog`); the session loop
    /// goes on with pictures and acknowledgements meanwhile, and
    /// `settle_input` takes the result and starts the next one.
    pub(super) async fn input(&mut self, events: &[InputEvent]) -> io::Result<()> {
        if self.health.injecting.is_some() || !self.health.backlog.is_empty() {
            // Round 8 (I2): a batch of only moves and wheel steps joins one
            // waiting before it, while nothing is pressed (round 10: a drag
            // keeps its exact path).
            let free = self.pressed.is_empty() && !self.presses_pending();
            let merged = match self.health.backlog.back_mut() {
                Some(waiting)
                    if free && inject::motion_only(waiting) && inject::motion_only(events) =>
                {
                    inject::merge_motion(waiting, events)
                }
                _ => false,
            };
            if merged {
                return Ok(());
            }
            let waiting: usize = self.health.backlog.iter().map(Vec::len).sum();
            if !self.health.overflow && waiting + events.len() <= inject::BACKLOG_EVENTS {
                self.health.backlog.push_back(events.to_vec());
                return Ok(());
            }
            // Round 9 (E6), round 10: beyond the bound only releases wait,
            // from this batch and every next one until the backlog drained
            // (`overflow`), so no later press meets the dropped ones.
            self.health.overflow = true;
            let releases = self.overflow_releases(events);
            if !releases.is_empty() {
                self.health.backlog.push_back(releases);
            }
            return self.report_held(InputHeld::BacklogFull).await;
        }
        self.health.overflow = false;
        self.start_batch(events).await
    }

    /// The checks of a batch, then its injection off the session loop.
    async fn start_batch(&mut self, events: &[InputEvent]) -> io::Result<()> {
        if self.family.selected.is_some() {
            self.release_pressed();
            return self
                .error(
                    ScreenErrorCode::ControlUnavailable,
                    "family_input_requires_snapshot_target".into(),
                )
                .await;
        }
        let sink = self.pipeline.input_sink();
        let reference = self.pipeline.input_reference_generation();
        let presented = self.pipeline.presentation_supported();
        // Round 6: every branch that does not inject the batch still lets
        // go of what the batch releases of this session's held keys.
        if let Some(held) = self.hold_reason() {
            self.release_held_ups(events);
            return self.report_held(held).await;
        }
        if !presented
            && sink
                .as_ref()
                .is_some_and(|sink| sink.legacy_input_invalid())
        {
            self.release_pressed();
            return self.error(ScreenErrorCode::ControlUnavailable,
                "Der Bildausschnitt hat sich geändert. Aufnahme läuft weiter; zum Steuern ist eine neue Verbindung erforderlich.".into()).await;
        }
        // W3: a session without the control injects nothing, old releases
        // neither (`release_control` sent them once when it let go).
        if !self.handle.holds_control() {
            self.release_held_ups(events);
            let detail = "input without control".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        if let Some(blocked) = self.release_blocked.take() {
            // D7, E2: the failed releases, the queued ones and this batch's
            // releases of held keys go in one job (`flush_releases`).
            self.release_held_ups(events);
            return self.report_block(blocked).await;
        }
        // D7: owed releases go out before the next batch (that batch waits
        // behind them like behind every release, and the viewer hears so, N2).
        if self.releases_owed() {
            self.release_held_ups(events);
            return self.report_held(InputHeld::ReleasePending).await;
        }
        let owned = events.to_vec();
        let (backend, target) = (self.context.backend.clone(), sink);
        let handle = tokio::task::spawn_blocking(move || match target {
            Some(sink) => sink.inject(&owned, reference, presented),
            None => backend.inject(&owned),
        });
        self.health.injecting = Some(super::input_release::Injection {
            handle,
            events: events.to_vec(),
            reference,
            presented,
            started: Instant::now(),
            epoch: self.health.release_epoch,
        });
        Ok(())
    }

    pub(super) async fn control(&mut self, action: ControlAction) -> io::Result<()> {
        if self.mode != SessionMode::Control {
            let detail = "this session may only view".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        match action {
            ControlAction::Take => match self.handle.take_control() {
                Ok(()) => {
                    let audit = self.context.audit;
                    log_event(
                        audit,
                        self.principal,
                        self.mode,
                        "control_taken",
                        None,
                        None,
                    );
                }
                Err(holder) => {
                    let state = ControlState {
                        holder: Some(holder),
                        blocked: None,
                        held: None,
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                    return self
                        .error(ScreenErrorCode::ControlBusy, String::new())
                        .await;
                }
            },
            ControlAction::Release => self.release_control().await,
        }
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: None,
            held: None,
        };
        self.health.block_reported = None;
        self.health.held_reported = None;
        self.frame(HostFrame::ControlState(state)).await
    }

    /// Lets go of every pressed input and of the control. Z3/Z4: a release
    /// job still running ends first, then everything held now and every
    /// failed release goes in one job, and only after that job (at most
    /// `CLOSE_GRACE` each) another session may take the control.
    pub(super) async fn release_control(&mut self) {
        // Round 6 (R1): everything held, queued and failed goes before
        // another session may take the control.
        self.drain_releases().await;
        // Codex 7/1: what is still on its way goes to the release keeper,
        // which holds every session's input until it is delivered; the
        // control goes at once.
        self.hand_releases_over();
        self.give_up_control();
    }

    /// The end of the session: everything held and owed goes as for
    /// `release_control`, then the control goes in any case, with its audit
    /// entry (round 7: the handle's drop gave it back without one); releases
    /// still on their way are named in the host log.
    pub(super) async fn end_control(&mut self) {
        self.drain_releases().await;
        self.hand_releases_over();
        self.give_up_control();
    }

    /// The control goes back to the registry (after `release_control`).
    pub(super) fn give_up_control(&mut self) {
        if self.handle.release_control() {
            let audit = self.context.audit;
            log_event(
                audit,
                self.principal,
                self.mode,
                "control_released",
                None,
                None,
            );
        }
    }
}

impl Session<'_> {
    /// A metadata request into the queue. A full queue (its answers wait
    /// for the outbox, round 9 F9) answers at once instead of holding the
    /// viewer's frames back, which held its input back too: an icon is
    /// omitted, as when its lookup fails; a list ends in an `ERROR` and the
    /// session goes on.
    async fn push_metadata(
        &mut self,
        request: crate::screen::core::host_metadata::Request,
    ) -> io::Result<()> {
        use crate::screen::core::host_metadata::Request;
        if self.metadata.can_accept() {
            return self.metadata.push(self.context, self.principal, request);
        }
        match request {
            Request::Icon(request) => {
                let reply = crate::screen::core::app_icon::IconResponse {
                    request,
                    content: None,
                };
                self.frame(HostFrame::IconReply(reply)).await
            }
            Request::Windows | Request::Families => {
                self.error(
                    ScreenErrorCode::CaptureFailed,
                    "metadata_queue_full: the viewer asked faster than the highest data rate carries"
                        .to_string(),
                )
                .await
            }
        }
    }
}

impl Session<'_> {
    /// Sound that does not fit under the viewer's ceiling is not sent; the
    /// viewer hears why once (round 8 Gemini 8/10).
    async fn sound_notice(
        &mut self,
        ceiling: Option<Option<u64>>,
        on: Option<bool>,
    ) -> io::Result<()> {
        match self.outbox.sound.update(ceiling, on) {
            Some(text) => self.report(self.state, Some(&text)).await,
            None => Ok(()),
        }
    }
}

#[path = "host_session_inject.rs"]
mod inject;
#[path = "host_session_report.rs"]
mod report;
