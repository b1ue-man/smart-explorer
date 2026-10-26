//! ScreenSource reference, cache, input and final native-resource ownership.
use super::super::x11_input;
use super::*;

impl ScreenSource for X11Source {
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.grant.clone()
    }
    fn legacy_input_invalid(&self) -> bool {
        self.geometry_changed
    }
    fn reference_generation(&self) -> u64 {
        self.reference_generation
    }
    fn inject_input(
        &mut self,
        _backend: &dyn crate::screen::core::host_backend::ScreenHostBackend,
        reference: u64,
        presented: bool,
        events: &[crate::screen::core::input_model::InputEvent],
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        use crate::screen::core::wire_control::ControlBlock;
        if reference != self.reference_generation {
            return Err(ControlBlock::ControlUnavailable);
        }
        let current = self
            .current_area()
            .map_err(|_| ControlBlock::ControlUnavailable)?;
        if self.geometry != Some(current) {
            self.geometry_changed = true;
            return Err(ControlBlock::ControlUnavailable);
        }
        if self.last_image.is_none() || (!presented && self.geometry_changed) {
            return Err(ControlBlock::ControlUnavailable);
        }
        x11_input::inject_bound(&self.source, current, events)
    }
    fn frame_limits(&self) -> FrameLimits {
        FrameLimits::RESOURCE
    }
    fn size(&self) -> FrameSize {
        self.size
    }

    fn capture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        let (mut image, x, y) = match self.pass()? {
            Pass::Paused(state) => return Ok(SourceFrame::Paused(state)),
            Pass::Image(image, x, y) => (image, x, y),
        };
        if self.cursor {
            if let Ok(cursor) = x11_cursor::fetch(
                &self.conn,
                &self.gate,
                self.grant
                    .as_ref()
                    .ok_or_else(|| failed("capture_not_activated"))?,
                self.admission.authority(),
            ) {
                let size = self.size;
                x11_cursor::blend(image.pixels_mut(), size, x, y, &cursor.image);
            }
        }
        let image = image.freeze();
        let captured = x11_input::Area {
            x,
            y,
            width: image.size().width as i32,
            height: image.size().height as i32,
        };
        let current = self.current_area()?;
        self.admit_geometry(current)?;
        if current != captured {
            return Ok(SourceFrame::Paused(SessionState::Starting));
        }
        let hash = picture_hash(image.image().data());
        let same = self.last_hash == Some(hash)
            && self.last_image.as_ref().is_some_and(|last| {
                last.size() == image.size() && last.image().data() == image.image().data()
            });
        self.last_hash = Some(hash);
        if !same {
            self.last_image = Some(image.clone());
        }
        if same {
            return if keyframe {
                Ok(SourceFrame::Frame(
                    self.last_image
                        .clone()
                        .ok_or_else(|| failed("missing unchanged cache"))?,
                ))
            } else {
                Ok(SourceFrame::Unchanged)
            };
        }
        Ok(SourceFrame::Frame(image))
    }
}
impl Drop for X11Source {
    fn drop(&mut self) {
        if let Some(segment) = self.segment.take() {
            segment.release(&self.conn);
        }
        self.last_image = None;
        if let Some(grant) = &self.grant {
            if !grant.is_member() {
                if let Err(error) = grant.retire() {
                    eprintln!("X11 capture retirement: {error}");
                }
            }
        }
    }
}
