//! Session and host events for the desktop viewer; decoding owns the same state.
use super::*;

impl DecodeState {
    /// Applies one event; `true` when it was the session's last.
    pub fn on_event(&mut self, event: StreamEvent) -> bool {
        if self
            .shared
            .presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed
        {
            self.gop = None;
            return true;
        }
        self.check_awaiting();
        self.poll_switch();
        self.guard_tick();
        match event {
            StreamEvent::Pending(_) => self
                .shared
                .update(|info| info.state = SessionState::WaitingForApproval),
            StreamEvent::Accepted {
                hello,
                path,
                rate_ceiling,
                set_decoders,
            } => {
                let exceeds = self
                    .shared
                    .memory
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .is_some_and(|memory| {
                        hello
                            .frame_contract
                            .is_some_and(|contract| contract.limits.working_bytes > memory.bytes())
                    });
                if hello.validate().is_err() || exceeds {
                    self.connection_lost();
                    return true;
                }
                if self.shared.info().family.is_some()
                    && (hello.family_version != Some(1)
                        || hello.frame_contract.is_none_or(|contract| {
                            contract.version != crate::screen::core::frame_budget::FRAME_PROTOCOL
                        }))
                {
                    self.shared.end(EndInfo {
                        text: "Der Host bestätigt den Familien-/Bildvertrag nicht.".into(),
                        code: None,
                    });
                    return true;
                }
                {
                    let icons = self.shared.info().icons.clone();
                    icons
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .requests
                        .negotiate(
                            hello.icon_version == Some(crate::screen::core::app_icon::ICON_VERSION),
                        );
                }
                // P1: a new „Codec“/„Dekodierer“ goes out as `SET_DECODERS`.
                self.shared
                    .local
                    .set_decoders
                    .store(set_decoders, std::sync::atomic::Ordering::Release);
                self.shared.update(|info| {
                    info.hello = Some(hello);
                    info.path = Some(path);
                    info.rate_ceiling = rate_ceiling;
                    info.accepted += 1;
                    if matches!(
                        info.state,
                        SessionState::Connecting | SessionState::WaitingForApproval
                    ) {
                        info.state = SessionState::Starting;
                    }
                });
                // The stored „Verlustfrei“/„Scharf (GPU)“ (user rule
                // 2026-11-18): only `SET_MODE` carries them to the host.
                if let Some(mode) = self.shared.opening_mode() {
                    let _ = (self.send)(&ViewerFrame::SetMode(mode));
                }
                // A chosen „Dekodier-Bibliothek“ that is absent is named,
                // never replaced by the other one.
                if let Some(library) = crate::screen::core::decoder_library::chosen_missing() {
                    let text = crate::screen::core::decoder_library::missing_text(library);
                    self.shared.update(|info| info.notice = Some(text));
                }
                if self.take_control {
                    let take = ControlRequest {
                        action: ControlAction::Take,
                    };
                    let _ = (self.send)(&ViewerFrame::ControlRequest(take));
                }
                // The overlay pointer (P6): a new connection starts without it.
                self.shared.update(|info| info.cursor = Default::default());
                if self.shared.wants_cursor() {
                    // P8: off while the host draws it into a Direct3D 11 picture.
                    let wish = self.shared.direct.cursor_overlay();
                    let _ = (self.send)(&ViewerFrame::CursorOverlay(wish));
                }
            }
            StreamEvent::Host(frame) => {
                if let Some(result) =
                    super::super::family_viewer_events::receive(&self.shared, &self.send, &frame)
                {
                    match result {
                        Err(error) => {
                            self.shared.end(EndInfo {
                                text: error.to_string(),
                                code: None,
                            });
                            return true;
                        }
                        Ok(true) => {
                            self.shared.cancel_presentation();
                            self.chain.continuity.broken();
                            self.gop = None;
                            self.request_keyframe(KeyframeReason::Reset, self.latest_gop);
                        }
                        Ok(false) => {}
                    }
                } else {
                    self.host(frame);
                }
            }
            StreamEvent::Record { header, record } => self.record(header, record),
            StreamEvent::Broken { gop_id } => {
                if gop_id < self.latest_gop {
                    return false;
                }
                // Only the rest of the group is lost: the shown picture stays
                // (without receipts, input blocked) until the key frame, or
                // (B-36) until the continuation the host sends after it
                // reset the stream; the decoder then stays.
                self.shared.suspend_presentation();
                let waits = self.waits_for_stream(gop_id);
                if !waits && self.gop.as_ref().is_some_and(|gop| gop.id == gop_id) {
                    self.gop = None;
                }
                self.broken = Some(gop_id);
                self.broken_frame = self
                    .last_record
                    .filter(|(group, _)| *group == gop_id)
                    .map(|(_, frame)| frame);
                if !waits {
                    self.request_keyframe(KeyframeReason::Reset, gop_id);
                }
            }
            StreamEvent::Clock(offset) => self.shared.update(|info| info.clock_offset_us = offset),
            // A „Höchste Datenrate“ changed during the session (P1): the
            // statistics say whether the host took it.
            StreamEvent::RateCeiling(state) => self.shared.update(|info| info.rate_ceiling = state),
            StreamEvent::End(end) => {
                self.gop = None;
                self.shared.stop_presentation();
                let text = end_text(&end.code, end.detail.as_deref());
                self.shared.update(|info| {
                    info.end = Some(EndInfo {
                        text,
                        code: Some(end.code),
                    });
                    info.state = SessionState::Ended;
                });
                return true;
            }
        }
        false
    }

    /// The reading side broke without an end from the relay.
    pub fn connection_lost(&mut self) {
        self.gop = None;
        self.shared.end(EndInfo {
            text: end_text("connection_lost", None),
            code: Some("connection_lost".into()),
        });
    }

    pub(super) fn host(&mut self, frame: HostFrame) {
        match frame {
            HostFrame::State(report) => {
                if report.state != SessionState::Streaming
                    && self.shared.info().state != report.state
                {
                    self.shared.suspend_presentation();
                }
                // Every change of the host's encoder and a chosen codec it
                // cannot keep are named (user rule 2026-11-18).
                let chosen = self.shared.codec_choice();
                if let Some(stream) = report.stream.as_ref() {
                    self.shared.note_stream(stream.codec);
                }
                self.shared.update(|info| {
                    info.state = report.state;
                    if let Some(text) = report.reason.as_deref().and_then(|reason| {
                        super::super::screen_texts::codec_reason_text(reason, Some(chosen))
                    }) {
                        info.notice = Some(text);
                    }
                    info.reason = report.reason;
                    if report.stream.is_some() {
                        info.stream = report.stream;
                    }
                })
            }
            HostFrame::Error(error) => {
                let text = error_text(&error.code, error.detail.as_deref());
                let busy = error.code == "control_busy";
                self.shared.update(|info| {
                    info.notice = Some(text);
                    info.control_denied |= busy;
                });
            }
            HostFrame::Windows(list) => {
                self.shared.update(|info| {
                    info.icons
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .requests
                        .refreshed();
                    info.windows = Some(list);
                });
            }
            HostFrame::IconReply(response) => {
                let (icons, peer) = {
                    let info = self.shared.info();
                    (info.icons.clone(), info.icon_peer.clone())
                };
                icons
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .receive(&peer, &response);
            }
            HostFrame::ControlState(state) => self.shared.update(|info| {
                info.controller = state.holder;
                info.blocked = state.blocked;
            }),
            HostFrame::Stats(stats) => {
                // BK0: a host announcing `frame_timing_v1` is asked for its
                // station times, again every `TIMING_RENEW`.
                let ask = stats.frame_timing
                    == Some(crate::screen::core::frame_timing::FRAME_TIMING_VERSION)
                    && self.shared.info().latency.timing_due();
                self.shared.update(|info| info.stats = Some(stats));
                if ask {
                    let request = crate::screen::core::frame_timing::FrameTimingRequest::start();
                    let _ = (self.send)(&ViewerFrame::FrameTiming(request));
                }
            }
            HostFrame::CursorShape(shape) => {
                self.shared.update(|info| info.cursor.set_shape(shape))
            }
            HostFrame::CursorPosition(position) => self
                .shared
                .update(|info| info.cursor.set_position(position)),
            HostFrame::Families(_)
            | HostFrame::FamilySnapshot(_)
            | HostFrame::FamilyOperation(_)
            | HostFrame::FamilyInput(_) => {}
            // The relay answers `PING` and reports the clock itself.
            HostFrame::Pong(_) => {}
            HostFrame::VisibilityAck(ack) => {
                self.shared
                    .visibility_ack(ack.geometry_generation, ack.min_frame_id);
            }
        }
    }
}
