//! Visibility and explicit source changes share the reference/input boundary.
use super::*;

impl Session<'_> {
    pub(super) async fn visibility(&mut self, report: crate::screen::core::visibility::ViewerVisibility) -> io::Result<()> {
        use crate::screen::core::visibility::VisibilityState;
        report.validate()?;
        if !self.visibility_supported {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "visibility was not negotiated"));
        }
        if report.geometry_generation <= self.visibility_generation { return Ok(()); }
        self.visibility_generation = report.geometry_generation;
        self.visible = !report.hidden();
        self.release_pressed();
        self.pipeline.begin_capture_active(self.visible)?;
        self.pipeline.set_visible_rects(if report.state == VisibilityState::Visible {
            Some(report.rects)
        } else { None });
        self.pipeline.invalidate_reference();
        let floor = self.pipeline.next_frame_id();
        self.input_floor = Some(floor);
        self.visibility_ack_pending = true;
        Ok(())
    }

    pub(super) async fn set_mode(&mut self, set: SetMode) -> io::Result<()> {
        if let Some(source) = set.source.as_ref() {
            let selected=crate::screen::core::family_protocol::selector(source).map(str::to_owned);
            if selected.is_some() && !self.family.supported {
                return Err(io::Error::new(io::ErrorKind::Unsupported,"family_contract_not_negotiated"));
            }
            self.release_pressed();
            let content = source.is_screen_content();
            let backend = self.context.backend.as_ref();
            if content && !(backend.graphical_session() && self.context.indicator.show().is_ok()) {
                let detail = format!("source {source}");
                return self
                    .error(ScreenErrorCode::IndicatorUnavailable, detail)
                    .await;
            }
            match crate::screen::core::family_listing::open(self.context,self.principal,source,None,self.mode==SessionMode::Control) {
                Ok(opened) => {
                    if self.handle.stop_signal().borrow().is_some()
                        || !crate::screen::core::family_listing::admitted(self.context,self.principal,self.mode==SessionMode::Control) {
                        drop(opened);return Err(io::Error::new(io::ErrorKind::PermissionDenied,"source_permission_revoked"));
                    }
                    self.pipeline.set_source(opened);
                    self.family.reset(selected);
                    self.input_floor=Some(self.pipeline.next_frame_id());
                    self.handle.set_screen_content(content);
                    let name = source.to_string();
                    self.handle.update(|view| view.source = name);
                }
                Err(refusal) => {
                    return self.error(ScreenErrorCode::NoDisplay, refusal.reason).await;
                }
            }
        }
        let kind = match set.mode {
            StreamMode::Preview => StreamKind::Preview,
            StreamMode::Full => StreamKind::Video,
        };
        self.pipeline.set_mode(kind, set.size, set.fps, set.sharpness, set.lossless);
        let shown = match (kind, self.mode) {
            (StreamKind::Preview, _) => SessionMode::Preview,
            (_, SessionMode::Control) => SessionMode::Control,
            _ => SessionMode::View,
        };
        self.handle.update(|view| view.mode = shown);
        if self.family.selected.is_some() {self.family_snapshot(true).await?;}
        Ok(())
    }

    pub(super) fn refresh_input_reference(&mut self) {
        let floor=self.pipeline.reference_floor();
        let changed=floor!=self.observed_reference_floor;
        self.observed_reference_floor=floor;
        if self.visible && !self.visibility_ack_pending {
            if self.pipeline.presentation_supported() && self.pipeline.input_reference_ready() {self.input_floor=None;}
            if !self.pipeline.presentation_supported() && self.pipeline.known_ack().is_some_and(|ack|
                self.input_floor.is_some_and(|floor|ack>=floor) && ack<self.pipeline.next_frame_id()) {self.input_floor=None;}
        }
        if changed || (self.pipeline.presentation_supported() && !self.pipeline.input_reference_ready()) {
            self.input_floor=Some(floor);
            self.release_pressed();
        }
    }
}
