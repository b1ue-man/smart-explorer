//! Viewer control frames, input authorization, control release and the
//! pointer overlay (P6) for one session.
use super::*;
use crate::screen::core::cursor_overlay::{CursorFeed, CursorPosition, CursorSeen, CursorUpdate};
use crate::screen::core::wire_control::InputHeld;

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
        self.refresh_input_reference();
        self.end_hold().await?;
        self.report_refusal().await?;
        self.finish_handover().await?;
        match frame {
            ViewerFrame::ListFamilies => {
                if !self.family.supported {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "family_contract_not_negotiated",
                    ));
                }
                self.metadata.push(
                    self.context,
                    self.principal,
                    crate::screen::core::host_metadata::Request::Families,
                )
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
            ViewerFrame::ListWindows => self.metadata.push(
                self.context,
                self.principal,
                crate::screen::core::host_metadata::Request::Windows,
            ),
            ViewerFrame::IconRequest(request) => {
                if !self.icons_supported {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "icon contract was not negotiated",
                    ));
                }
                request.validate()?;
                self.metadata.push(
                    self.context,
                    self.principal,
                    crate::screen::core::host_metadata::Request::Icon(request),
                )
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
                Ok(())
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

    pub(super) async fn input(&mut self, events: &[InputEvent]) -> io::Result<()> {
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
        let (backend, target) = (self.context.backend.clone(), sink.clone());
        let injecting = Instant::now();
        let applied = super::input_release::inject_off_runtime(move || match target {
            Some(sink) => sink.inject(&owned, reference, presented),
            None => backend.inject(&owned),
        })
        .await;
        // Station `input_inject` (BK0): the injection of this batch.
        let micros = u64::try_from(injecting.elapsed().as_micros()).unwrap_or(u64::MAX);
        crate::screen::core::chain_meter::lock(self.pipeline.chain_meter()).record_us(
            crate::screen::core::chain_meter::Station::InputInject,
            micros,
        );
        match applied {
            Ok(()) => {
                for event in events {
                    self.pressed.observe(event);
                }
                self.health.unconfirmed = None;
                // D6/E2: a block or hold the viewer heard ends with the
                // first batch that reached the host after it; a block the
                // platform reported late stays shown its minimum time (Z2).
                let held = std::mem::take(&mut self.health.held_reported);
                let late = self
                    .health
                    .block_shown_until
                    .is_some_and(|until| Instant::now() < until);
                let cleared = !late && self.health.block_reported.take().is_some();
                if cleared || held {
                    let state = ControlState {
                        holder: self.handle.controller(),
                        blocked: self.health.block_reported,
                        held: None,
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                }
                Ok(())
            }
            Err(blocked) => {
                // S3: a refusal for a reference the source just left holds
                // the batch back and keeps what the user holds; the new
                // reference lets go of it when it moves the floor.
                if presented && self.stale_refusal(blocked, reference) {
                    // Held back, not blocked (review O2a); its releases of
                    // held keys go in (E2: the viewer sends each once).
                    self.release_held_ups(events);
                    return self.report_held(InputHeld::WaitingForPicture).await;
                }
                self.release_pressed();
                self.report_block(blocked).await?;
                if !presented
                    && sink
                        .as_ref()
                        .is_some_and(|sink| sink.legacy_input_invalid())
                {
                    self.error(ScreenErrorCode::ControlUnavailable,
                        "Der Bildausschnitt hat sich geändert. Aufnahme läuft weiter; zum Steuern ist eine neue Verbindung erforderlich.".into()).await
                } else {
                    Ok(())
                }
            }
        }
    }

    pub(super) async fn control(&mut self, action: ControlAction) -> io::Result<()> {
        if self.mode != SessionMode::Control {
            let detail = "this session may only view".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        match action {
            ControlAction::Take => match self.handle.take_control() {
                Ok(()) => {
                    self.health.handover = None;
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
        self.health.held_reported = false;
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
        // Codex 6/5: releases still on their way keep the control here (the
        // viewer still sees itself holding it) until they are in, at most
        // `HANDOVER_LIMIT`; a new holder's first press cannot meet them.
        if self.input_release.is_some() || self.releases_owed() {
            self.health.handover.get_or_insert_with(Instant::now);
            return;
        }
        self.give_up_control();
    }

    /// The end of the session: everything held and owed goes as for
    /// `release_control`, then the control goes in any case, with its audit
    /// entry (round 7: the handle's drop gave it back without one); releases
    /// still on their way are named in the host log.
    pub(super) async fn end_control(&mut self) {
        self.drain_releases().await;
        if self.input_release.is_some() || self.releases_owed() {
            super::visibility::note_viewer("the session ends with releases still on their way");
        }
        self.give_up_control();
    }

    /// The control goes back to the registry (after `release_control`).
    fn give_up_control(&mut self) {
        self.health.handover = None;
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

#[path = "host_session_report.rs"]
mod report;
