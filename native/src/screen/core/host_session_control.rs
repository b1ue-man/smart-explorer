//! Viewer control frames, input authorization and control release for one session.
use super::*;

impl Session<'_> {
    pub(super) async fn on_frame(&mut self, frame: ViewerFrame) -> io::Result<()> {
        self.pipeline.poll();
        self.poll_input_release();
        self.refresh_input_reference();
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
                self.pipeline.request_keyframe(request.gop_id);
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
        }
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
        if !self.visible
            || self.input_floor.is_some()
            || !self.pipeline.input_reference_ready()
            || self.input_release.is_some()
        {
            return Ok(());
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
        if let Some(blocked) = self.release_blocked.take() {
            return self
                .frame(HostFrame::ControlState(ControlState {
                    holder: self.handle.controller(),
                    blocked: Some(blocked),
                }))
                .await;
        }
        if !self.handle.holds_control() {
            let detail = "input without control".to_string();
            return self.error(ScreenErrorCode::RightNotGranted, detail).await;
        }
        let owned = events.to_vec();
        let (backend, target) = (self.context.backend.clone(), sink.clone());
        let applied = super::input_release::inject_off_runtime(move || match target {
            Some(sink) => sink.inject(&owned, reference, presented),
            None => backend.inject(&owned),
        })
        .await;
        match applied {
            Ok(()) => {
                for event in events {
                    self.pressed.observe(event);
                }
                Ok(())
            }
            Err(blocked) => {
                self.release_pressed();
                let state = ControlState {
                    holder: self.handle.controller(),
                    blocked: Some(blocked),
                };
                self.frame(HostFrame::ControlState(state)).await?;
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
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                    return self
                        .error(ScreenErrorCode::ControlBusy, String::new())
                        .await;
                }
            },
            ControlAction::Release => self.release_control(),
        }
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: None,
        };
        self.frame(HostFrame::ControlState(state)).await
    }

    /// Lets go of every pressed input and of the control.
    pub(super) fn release_control(&mut self) {
        self.release_pressed();
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
