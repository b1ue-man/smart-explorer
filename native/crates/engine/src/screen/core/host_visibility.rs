//! Visibility and explicit source changes share the reference/input boundary;
//! a changed „Monitor“/„Auflösung“ (`SET_DISPLAY`, P9) changes the source too.
use super::*;
use crate::screen::core::window_list::SourceId;

/// A viewer message that starts the picture stream anew, into the screen
/// host log (measurement of 2026-11-19, as `Pipeline::note_restart`).
pub(super) fn note_viewer(why: &str) {
    let line = format!("screen stream: {why}");
    // The unit tests write no file of the account they run under.
    #[cfg(not(test))]
    crate::screen::os::host_worker_client::host_log::note(None, &line);
    #[cfg(test)]
    let _ = line;
}

impl Session<'_> {
    pub(super) async fn visibility(
        &mut self,
        report: crate::screen::core::visibility::ViewerVisibility,
    ) -> io::Result<()> {
        use crate::screen::core::visibility::VisibilityState;
        report.validate()?;
        if !self.visibility_supported {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "visibility was not negotiated",
            ));
        }
        if report.geometry_generation <= self.visibility_generation {
            return Ok(());
        }
        self.visibility_generation = report.geometry_generation;
        // A visible view that stays visible changed only its visible area
        // (an edge pan, a moved or resized window): the encoder, the capture
        // and the held keys stay (review 2026-11-19, V1/C3). With hybrid
        // regions too (Codex 2/3): their encoders take the rectangles from
        // every picture request and the key frame sends every region
        // (`HybridEncoder::encode_impl`, `captured_hybrid`). The
        // acknowledgement follows as for every report (`produce_with`).
        if self.visible && !report.hidden() {
            note_viewer(&format!(
                "the viewer reported its visibility (generation {}, {} rectangles): \
                 same encoder, key frame",
                report.geometry_generation,
                report.rects.len()
            ));
            let rects = (report.state == VisibilityState::Visible).then_some(report.rects);
            self.pipeline.retarget_visible_rects(rects)?;
            self.visibility_ack_pending = true;
            return Ok(());
        }
        note_viewer(&format!(
            "the viewer reported its visibility (generation {}, {} rectangles): new encoder",
            report.geometry_generation,
            report.rects.len()
        ));
        self.visible = !report.hidden();
        self.release_pressed();
        self.pipeline.begin_capture_active(self.visible)?;
        self.pipeline
            .set_visible_rects(if report.state == VisibilityState::Visible {
                Some(report.rects)
            } else {
                None
            });
        self.pipeline.invalidate_reference();
        let floor = self.pipeline.next_frame_id();
        self.input_floor = Some(floor);
        self.visibility_ack_pending = true;
        Ok(())
    }

    pub(super) async fn set_mode(&mut self, set: SetMode) -> io::Result<()> {
        // The session's own source named again (Android names it with every
        // setting) is no change of source (review 2026-11-19, C21).
        if let Some(source) = set.source.as_ref().filter(|s| !self.shows_source(s)) {
            if !self.change_source(Some(source.clone()), None).await? {
                return Ok(());
            }
        }
        let kind = match set.mode {
            StreamMode::Preview => StreamKind::Preview,
            StreamMode::Full => StreamKind::Video,
        };
        // P5 (plan 2026-11-17): the display follows the full view's window
        // size when this session claimed it; at most once a second.
        // The picture waits for the display's new size, so only that change
        // opens a new encoder (review 2026-11-19, Gemini 13.1).
        if set.mode == StreamMode::Full
            && crate::screen::core::virtual_display_host::resize(&self.handle.id(), set.size)
        {
            if let Some(size) = set.size {
                self.pipeline.expect_source_size(size);
            }
        }
        self.pipeline
            .set_mode(kind, set.size, set.fps, set.sharpness, set.lossless);
        let shown = match (kind, self.mode) {
            (StreamKind::Preview, _) => SessionMode::Preview,
            (_, SessionMode::Control) => SessionMode::Control,
            _ => SessionMode::View,
        };
        self.handle.update(|view| view.mode = shown);
        if self.family.selected.is_some() {
            self.family_snapshot(true).await?;
        }
        Ok(())
    }

    /// `SET_DISPLAY` (P9, `set_display.rs`): the display is decided again
    /// with the new wish, then the session's monitor source is prepared
    /// again, so capture and encoder open anew with a key frame. A session
    /// without a wish (no `live` in its `HELLO_OK`) changes nothing.
    pub(super) async fn set_display(
        &mut self,
        choice: crate::screen::core::virtual_display::VirtualDisplayChoice,
    ) -> io::Result<()> {
        self.change_source(None, Some(choice)).await.map(|_| ())
    }

    /// `source` is the one this session shows now (its view's `source`,
    /// written at the start and by every change of source below).
    fn shows_source(&self, source: &SourceId) -> bool {
        let wanted = source.to_string();
        let mut shown = false;
        // A read: the registry publishes only a changed view.
        self.handle.update(|view| shown = view.source == wanted);
        shown
    }

    /// Prepares a source and puts it in place of the session's one: `wanted`
    /// for `SET_MODE`, or, with `display`, the monitor the session shows
    /// after its display was decided again (both on the worker thread).
    /// `Ok(false)` when the session answered with `ERROR` instead or the
    /// display had nothing to decide.
    async fn change_source(
        &mut self,
        wanted: Option<SourceId>,
        display: Option<crate::screen::core::virtual_display::VirtualDisplayChoice>,
    ) -> io::Result<bool> {
        use crate::screen::core::virtual_display_host as virtual_display;
        let selected = wanted
            .as_ref()
            .and_then(crate::screen::core::family_protocol::selector)
            .map(str::to_owned);
        if selected.is_some() && !self.family.supported {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "family_contract_not_negotiated",
            ));
        }
        self.release_pressed();
        let own = wanted.is_some();
        // A display change always ends at a monitor.
        let content = wanted.as_ref().is_none_or(SourceId::is_screen_content);
        let backend = self.context.backend.as_ref();
        if content && !(backend.graphical_session() && self.context.indicator.show().is_ok()) {
            let detail = match &wanted {
                Some(source) => format!("source {source}"),
                None => "display".to_string(),
            };
            return self
                .error(ScreenErrorCode::IndicatorUnavailable, detail)
                .await
                .map(|()| false);
        }
        let (bank, output) = self.pipeline.capture_context()?;
        let admission = bank.new_source_session()?;
        let mut stop = self.handle.stop_signal();
        if stop.borrow().is_some() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        struct PreparedResult {
            source: SourceId,
            result: Result<
                Box<dyn crate::screen::core::host_backend::PreparedSource>,
                crate::screen::core::host_backend::SourceRefusal,
            >,
            _admission: std::sync::Arc<crate::screen::core::capture_admission::SessionAdmission>,
        }
        let context = self.context.clone();
        let principal = self.principal.clone();
        let control = self.mode == SessionMode::Control;
        let retained = admission.clone();
        let id = self.handle.id();
        let job = tokio::task::spawn_blocking(move || {
            let capture = retained.session().to_string();
            let source = match (wanted, display) {
                (Some(source), _) => {
                    virtual_display::rebind_capture(&id, &capture);
                    source
                }
                (None, Some(choice)) => {
                    let switched = virtual_display::switch(&id, choice, &capture)?;
                    crate::screen::core::set_display::display_source(
                        &context.backend.monitors(),
                        switched.monitor.as_deref(),
                        switched.shown(),
                    )?
                }
                (None, None) => return None,
            };
            let result = crate::screen::core::family_listing::prepare(
                &context,
                &principal,
                &source,
                None,
                control,
                retained.clone(),
            );
            Some(PreparedResult {
                source,
                result,
                _admission: retained,
            })
        });
        let prepared = tokio::select! {biased;
            _ = self.connection.closed() => return Err(io::Error::from(io::ErrorKind::Interrupted)),
            _ = stop.changed() => return Err(io::Error::from(io::ErrorKind::Interrupted)),
            prepared = job => prepared.map_err(io::Error::other)?,
        };
        let Some(prepared) = prepared else {
            return Ok(false);
        };
        let source = prepared.source;
        match prepared.result {
            Ok(prepared) => {
                let context = self.context.clone();
                let principal = self.principal.clone();
                let allowed = std::sync::Arc::new(move || {
                    crate::screen::core::family_listing::admitted(&context, &principal, control)
                });
                let opened = crate::screen::core::host_admission::pending(
                    prepared, admission, output, allowed,
                );
                if self.handle.stop_signal().borrow().is_some()
                    || !crate::screen::core::family_listing::admitted(
                        self.context,
                        self.principal,
                        self.mode == SessionMode::Control,
                    )
                {
                    drop(opened);
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "source_permission_revoked",
                    ));
                }
                self.pipeline.set_source(opened);
                self.family.reset(selected);
                self.input_floor = Some(self.pipeline.next_frame_id());
                self.handle.set_screen_content(source.is_screen_content());
                let name = source.to_string();
                self.handle.update(|view| view.source = name);
                let monitor = match &source {
                    SourceId::Monitor(id) => Some(id.clone()),
                    _ => None,
                };
                virtual_display::source_changed(&self.handle.id(), monitor);
                // W1: a source the viewer chose does not follow the display.
                virtual_display::source_chosen(&self.handle.id(), own);
                self.cursor_source_changed().await?;
                Ok(true)
            }
            Err(refusal) => self
                .error(ScreenErrorCode::NoDisplay, refusal.reason)
                .await
                .map(|()| false),
        }
    }

    pub(super) fn refresh_input_reference(&mut self) {
        let floor = self.pipeline.reference_floor();
        let changed = floor != self.observed_reference_floor;
        self.observed_reference_floor = floor;
        if self.visible && !self.visibility_ack_pending {
            if self.pipeline.presentation_supported() && self.pipeline.input_reference_ready() {
                self.input_floor = None;
            }
            if !self.pipeline.presentation_supported()
                && self.pipeline.known_ack().is_some_and(|ack| {
                    self.input_floor.is_some_and(|floor| ack >= floor)
                        && ack < self.pipeline.next_frame_id()
                })
            {
                self.input_floor = None;
            }
        }
        // Only a new reference moves the floor (`Pipeline::drop_encoder`; a
        // new picture group of the same one does not, review 2026-11-19
        // E1). A reference that is not ready for a moment (a running
        // operation after an interrupt) holds input in `input`, without
        // letting go of the held keys.
        if changed {
            self.input_floor = Some(floor);
            self.release_pressed();
        }
    }
}
