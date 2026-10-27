//! Viewer visibility and input resumption. Focus never determines visibility.
use super::screen_input_map::Geometry;
use crate::screen::core::visibility::{ViewerVisibility, VisibilityState, VisibleRect};
use eframe::egui::Rect;

/// OS facts. Rectangles are client-local physical pixels. `None` is honest
/// uncertainty (Wayland or failed/racing native geometry), never occlusion.
#[derive(Clone, Debug, Default)]
pub(crate) struct WindowVisibility {
    pub minimized: bool,
    pub shown: Option<bool>,
    pub rects: Vec<Rect>,
    pub exit_chord: bool,
}
impl WindowVisibility {
    pub(crate) fn hidden(&self) -> bool {
        self.minimized || self.shown == Some(false)
    }
}
#[derive(Default)]
pub(crate) struct VisibilityTracker {
    last: Option<ViewerVisibility>,
    geometry: Option<Geometry>,
    generation: u64,
}
impl VisibilityTracker {
    pub(crate) fn invalidate(&mut self) {
        self.last = None;
        self.geometry = None;
    }
    pub(crate) fn report(
        &mut self,
        window: &WindowVisibility,
        area: Rect,
        geometry: Option<&Geometry>,
        ppp: f32,
    ) -> Option<ViewerVisibility> {
        let mut rects = Vec::new();
        let mut state = VisibilityState::Unknown;
        if window.hidden() {
            state = VisibilityState::Hidden;
        } else if let Some(g) = geometry {
            let project = |visible: Rect| -> Option<VisibleRect> {
                let clipped = visible.intersect(area).intersect(g.rect);
                if !clipped.is_positive() {
                    return None;
                }
                let axis = |value: f32, start: f32, span: f32, pixels: u32, ceil: bool| {
                    let v = ((value - start) / span * pixels as f32).clamp(0.0, pixels as f32);
                    if ceil {
                        v.ceil() as u32
                    } else {
                        v.floor() as u32
                    }
                };
                let x = axis(
                    clipped.min.x,
                    g.rect.min.x,
                    g.rect.width(),
                    g.size.width,
                    false,
                );
                let y = axis(
                    clipped.min.y,
                    g.rect.min.y,
                    g.rect.height(),
                    g.size.height,
                    false,
                );
                let right = axis(
                    clipped.max.x,
                    g.rect.min.x,
                    g.rect.width(),
                    g.size.width,
                    true,
                );
                let bottom = axis(
                    clipped.max.y,
                    g.rect.min.y,
                    g.rect.height(),
                    g.size.height,
                    true,
                );
                Some(VisibleRect {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                })
            };
            if window.shown.is_some() {
                rects.extend(
                    window
                        .rects
                        .iter()
                        .filter_map(|r| project(Rect::from_min_max(r.min / ppp, r.max / ppp))),
                );
                state = if rects.is_empty() {
                    VisibilityState::Hidden
                } else {
                    VisibilityState::Visible
                };
            } else if let Some(rect) = project(area) {
                rects.push(rect);
            }
        }
        if rects.len() > crate::screen::core::visibility::MAX_VISIBLE_RECTS {
            state = VisibilityState::Unknown;
            rects.clear();
        }
        // The host reads state, minimized and the rectangles (in picture
        // pixels); a picture that only moves on screen changes none of them
        // and must not cost a new reference picture.
        let size_changed = self.geometry.as_ref().map(|g| g.size) != geometry.map(|g| g.size);
        let changed = size_changed
            || self.last.as_ref().is_none_or(|last| {
                last.state != state || last.minimized != window.minimized || last.rects != rects
            });
        self.geometry = geometry.copied();
        if !changed {
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        let report = ViewerVisibility {
            geometry_generation: self.generation,
            state,
            minimized: window.minimized,
            rects,
        };
        self.last = Some(report.clone());
        Some(report)
    }
}
/// Shared with decoder. The acknowledgement binds the fresh full frame to
/// the report; old queued decoder pictures cannot satisfy its floor.
#[derive(Default)]
pub(crate) struct ResumeGate {
    generation: u64,
    visible: bool,
    floor: Option<u64>,
    ready: bool,
    legacy: bool,
    latest: Option<u64>,
}
impl ResumeGate {
    pub(crate) fn report(&mut self, report: &ViewerVisibility, modern: bool) {
        let was_hidden = !self.visible;
        self.visible = report.state != VisibilityState::Hidden && !report.minimized;
        self.generation = report.geometry_generation;
        self.legacy = !modern;
        self.floor = None;
        self.latest = None;
        self.ready = if modern {
            false
        } else {
            self.ready && !was_hidden && self.visible
        };
    }
    pub(crate) fn acknowledge(&mut self, generation: u64, min_frame: u64) {
        if generation == self.generation && self.visible {
            self.floor = Some(min_frame);
            self.ready = self.latest.is_some_and(|frame| frame >= min_frame);
        }
    }
    pub(crate) fn block(&mut self) {
        self.ready = false;
        self.latest = None;
    }
    pub(crate) fn can_present(&self, frame: u64) -> bool {
        self.generation == 0
            || self.visible && (self.legacy || self.floor.is_some_and(|floor| frame >= floor))
    }
    pub(crate) fn invalidate(&mut self) {
        self.ready = false;
        self.floor = None;
        self.latest = None;
    }
    pub(crate) fn complete(&mut self, frame: u64) {
        self.latest = Some(self.latest.map_or(frame, |previous| previous.max(frame)));
        if self.can_present(frame) {
            self.ready = true;
        }
    }
    pub(crate) fn ready(&self) -> bool {
        (self.generation == 0 || self.visible) && self.ready
    }
}

#[cfg(test)]
#[path = "batches_visibility_task_tests.rs"]
mod batches_task_tests;
