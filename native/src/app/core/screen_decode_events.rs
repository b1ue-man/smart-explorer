//! Session and host events for the desktop viewer; decoding owns the same state.
use super::*;

impl DecodeState {
    /// Applies one event; `true` when it was the session's last.
    pub(crate) fn on_event(&mut self, event: StreamEvent) -> bool {
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
        match event {
            StreamEvent::Pending(_) => self
                .shared
                .update(|info| info.state = SessionState::WaitingForApproval),
            StreamEvent::Accepted { hello, path } => {
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
                self.shared.update(|info| {
                    info.hello = Some(hello);
                    info.path = Some(path);
                    info.accepted += 1;
                    if matches!(
                        info.state,
                        SessionState::Connecting | SessionState::WaitingForApproval
                    ) {
                        info.state = SessionState::Starting;
                    }
                });
                if self.take_control {
                    let take = ControlRequest {
                        action: ControlAction::Take,
                    };
                    let _ = (self.send)(&ViewerFrame::ControlRequest(take));
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
                self.shared.cancel_presentation();
                if self.gop.as_ref().is_some_and(|gop| gop.id == gop_id) {
                    self.gop = None;
                }
                self.broken = Some(gop_id);
                self.broken_frame = self
                    .last_record
                    .filter(|(group, _)| *group == gop_id)
                    .map(|(_, frame)| frame);
                self.request_keyframe(KeyframeReason::Reset, gop_id);
            }
            StreamEvent::Clock(offset) => self.shared.update(|info| info.clock_offset_us = offset),
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
    pub(crate) fn connection_lost(&mut self) {
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
                self.shared.update(|info| {
                    info.state = report.state;
                    if let Some(reason) = report
                        .reason
                        .as_deref()
                        .and_then(|value| value.strip_prefix("encoder_fallback_to_tiles: "))
                    {
                        info.notice = Some(format!(
                            "Videoencoder nicht verfügbar; verlustfreie Übertragung. {reason}"
                        ));
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
            HostFrame::Stats(stats) => self.shared.update(|info| info.stats = Some(stats)),
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
