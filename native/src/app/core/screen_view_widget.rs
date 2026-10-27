//! The picture area of the desktop viewer (SCR-M7, U25-5/7/8): it takes the
//! newest picture of a session, draws it (YUV planes through the GPU
//! callback, BGRA through an egui texture), keeps the statistics in a corner
//! and, while „Steuern“ is on and the window has the focus, turns the
//! window's input into `INPUT` frames. Nothing here waits for a decoder or a
//! socket: the picture comes out of a slot, frames go into a queue.
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, EventFilter, FontId, Pos2, Rect, Sense, Vec2};

use super::screen_input_keys::{local_keys, LocalKey};
use super::screen_input_map::{Geometry, InputMapper};
use super::screen_prefs::Fit;
use super::screen_session::{Picture, PictureData, ViewerSession};
use super::screen_stats_overlay;
use super::screen_texture_cpu::CpuTexture;
use super::screen_wgpu_yuv::YuvFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_control::ViewerFrame;

/// Distance from the edge of the area at which a 1:1 picture larger than the
/// area starts to scroll, and how fast, in points.
const EDGE_ZONE: f32 = 24.0;
const EDGE_SPEED: f32 = 900.0;

/// What the viewer app tells the widget for one frame.
pub(crate) struct ViewParams<'a> {
    pub fit: Fit,
    /// „Steuern“ is on: window input goes to the host.
    pub control: bool,
    /// The session does not stream right now: the last picture is dimmed.
    pub dimmed: bool,
    /// Shown in the middle while no picture exists.
    pub status: &'a str,
    pub stats: Option<&'a [String]>,
    /// Painted over the picture; never changes the layout.
    pub hint: Option<&'a str>,
}

/// What the widget tells the viewer app.
pub(crate) struct ViewResult {
    /// The area in physical pixels (the size to ask the host for).
    pub display: FrameSize,
    pub picture: Option<FrameSize>,
    /// Shortcuts the viewer keeps for itself.
    pub locals: Vec<LocalKey>,
    /// Esc was pressed (leaves full screen while control is off).
    pub escape: bool,
}

pub(super) enum Current {
    Cpu(FrameSize),
    Gpu(Arc<YuvFrame>, super::screen_presented::PresentedFrame),
}

impl Current {
    fn size(&self) -> FrameSize {
        match self {
            Self::Cpu(size) => *size,
            Self::Gpu(frame, _) => frame.size,
        }
    }
}

pub(crate) use super::screen_viewport::image_rect;

/// The pan after `dt` seconds with the pointer at `hover`: toward the edge it
/// is near, while the picture is larger than the area there.
fn edge_pan(pan: Vec2, area: Rect, dest: Vec2, hover: Option<Pos2>, dt: f32) -> Vec2 {
    let Some(hover) = hover.filter(|pos| area.contains(*pos)) else {
        return pan;
    };
    let axis = |pan: f32, pos: f32, start: f32, end: f32, picture: f32, room: f32| {
        if picture <= room {
            return 0.0;
        }
        let step = EDGE_SPEED * dt;
        let moved = if pos - start < EDGE_ZONE {
            pan - step
        } else if end - pos < EDGE_ZONE {
            pan + step
        } else {
            pan
        };
        moved.clamp(0.0, picture - room)
    };
    Vec2::new(
        axis(pan.x, hover.x, area.min.x, area.max.x, dest.x, area.width()),
        axis(
            pan.y,
            hover.y,
            area.min.y,
            area.max.y,
            dest.y,
            area.height(),
        ),
    )
}

/// The picture area of one session.
pub(crate) struct ViewWidget {
    cpu: CpuTexture,
    current: Option<Current>,
    source_stamp: Option<super::screen_presented::PresentedFrame>,
    source_geometry: Option<FrameSize>,
    pan: Vec2,
    mapper: Option<InputMapper>,
    /// The GPU path takes the planes; cleared when a picture did not fit it.
    gpu: bool,
    was_focused: bool,
    visibility: super::screen_visibility::VisibilityTracker,
}

impl ViewWidget {
    pub(crate) fn new(gpu: bool) -> Self {
        Self {
            cpu: CpuTexture::default(),
            current: None,
            source_stamp: None,
            source_geometry: None,
            pan: Vec2::ZERO,
            mapper: None,
            gpu,
            was_focused: true,
            visibility: super::screen_visibility::VisibilityTracker::default(),
        }
    }

    fn accept(&mut self, ctx: &egui::Context, session: &ViewerSession, picture: Picture) {
        if self
            .source_stamp
            .as_ref()
            .is_some_and(|old| !old.same_source(&picture.presented))
            || matches!(
                (&self.current, &picture.data),
                (Some(Current::Cpu(_)), PictureData::Yuv(_))
                    | (Some(Current::Gpu(..)), PictureData::Bgra(_))
            )
        {
            self.cpu.reset();
            self.current = None;
        }
        self.source_stamp = Some(picture.presented.clone());
        self.source_geometry = Some(picture.size);
        let limits = session
            .info()
            .hello
            .as_ref()
            .and_then(|hello| hello.frame_contract)
            .map(|c| c.limits)
            .unwrap_or(crate::screen::core::frame_budget::FrameLimits::LEGACY);
        if limits.check(picture.size).is_err() {
            self.retire();
            return;
        }
        match picture.data {
            PictureData::Bgra(image) => {
                super::screen_render_caps::bind_texture(
                    &mut self.cpu,
                    session,
                    picture.memory.clone(),
                );
                self.cpu.update_frame(ctx, &image, picture.presented);
                self.current = Some(Current::Cpu(picture.size));
            }
            PictureData::Yuv(planes) => {
                let mut frame = YuvFrame::new(picture.size, planes);
                frame.memory = picture.memory;
                let fits_texture = super::screen_render_owner::get(ctx)
                    .and_then(|owner| owner.texture_limit())
                    .is_some_and(|limit| frame.size.width <= limit && frame.size.height <= limit);
                if self.gpu && frame.fits() && fits_texture {
                    self.current = Some(Current::Gpu(Arc::new(frame), picture.presented));
                } else {
                    // The next pictures come as BGRA.
                    self.gpu = false;
                    self.cpu.reset();
                    self.current = None;
                    self.source_stamp = None;
                    session.set_prefer_yuv(false);
                }
            }
        }
    }

    /// Sends everything the mapper holds, in batches.
    fn flush(&mut self, session: &ViewerSession, all: bool) -> bool {
        let Some(mapper) = self.mapper.as_mut() else {
            return false;
        };
        while let Some(batch) = mapper.take_batch() {
            session.send(ViewerFrame::Input(batch));
            if !all {
                break;
            }
        }
        mapper.has_backlog()
    }

    /// Lifts whatever the host still holds down for this viewer (control
    /// ends, the session ends); the lifts leave before any later frame.
    pub(crate) fn release_input(&mut self, session: &ViewerSession) {
        if let Some(mapper) = self.mapper.as_mut() {
            mapper.release_all();
        }
        self.flush(session, true);
    }

    pub(crate) fn source_changed(&mut self, session: &ViewerSession) {
        self.release_input(session);
        session.invalidate_input();
        let _ = session.take_picture();
        self.current = None;
        self.cpu.reset();
        self.source_stamp = None;
        self.source_geometry = None;
        self.pan = Vec2::ZERO;
        self.visibility.invalidate();
    }

    pub(crate) fn retire(&mut self) {
        self.current = None;
        self.source_stamp = None;
        self.source_geometry = None;
        self.cpu.reset();
        self.visibility.invalidate();
    }
    pub(crate) fn take_texture(&mut self) -> CpuTexture {
        self.retire();
        std::mem::take(&mut self.cpu)
    }
    pub(crate) fn adopt_texture(&mut self, texture: CpuTexture) {
        self.retire();
        self.cpu = texture;
    }
    pub(crate) fn reconnect(&mut self, gpu: bool) {
        self.retire();
        self.gpu = gpu;
        self.pan = Vec2::ZERO;
        self.mapper = None;
        self.was_focused = true;
    }

    /// The mapper, made on demand (tests).
    #[cfg(test)]
    pub(crate) fn mapper_mut(
        &mut self,
        caps: crate::screen::core::input_model::InputCapabilities,
    ) -> &mut InputMapper {
        self.mapper.get_or_insert_with(|| InputMapper::new(caps))
    }

    pub(crate) fn unmapped_keys(&self) -> u64 {
        self.mapper
            .as_ref()
            .map_or(0, |mapper| mapper.unmapped_keys)
    }

    /// Draws the area and handles its input.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &ViewerSession,
        params: &ViewParams,
    ) -> ViewResult {
        let ctx = ui.ctx().clone();
        let mut window = ctx
            .data(|data| {
                data.get_temp::<super::screen_visibility::WindowVisibility>(egui::Id::new(
                    "screen_window_visibility",
                ))
            })
            .unwrap_or_default();
        if window.hidden() || session.is_ended() || !ui.clip_rect().is_positive() {
            self.release_input(session);
            self.current = None;
            self.cpu.reset();
            self.source_stamp = None;
            self.source_geometry = None;
        }
        if self
            .source_stamp
            .as_ref()
            .is_some_and(|frame| !frame.active())
        {
            self.current = None;
            self.source_stamp = None;
            self.cpu.reset();
            // Keep measured source geometry while waiting for the visibility ACK's frame.
        }
        session.refresh_visibility();
        if let Some(picture) = session.take_picture() {
            if !window.hidden() && !session.is_ended() {
                if session.can_present(picture.presented.clone()) {
                    self.accept(&ctx, session, picture);
                } else {
                    session.keep_if_awaiting_floor(picture);
                }
            }
        }
        let ppp = ctx.pixels_per_point();
        let available = ui.available_size();
        let (area, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        if !ui.is_rect_visible(area) || !area.intersect(ui.clip_rect()).is_positive() {
            window.shown = Some(false);
            self.release_input(session);
            self.current = None;
            self.cpu.reset();
            self.source_stamp = None;
            self.source_geometry = None;
        }
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, Color32::BLACK);
        let size = self
            .current
            .as_ref()
            .map(Current::size)
            .or(self.source_geometry);
        let mut shown = None;
        let mut reported_geometry = None;
        if let Some(size) = size.filter(|size| size.width > 0 && size.height > 0) {
            if params.fit == Fit::Actual {
                let dest = Vec2::new(size.width as f32 / ppp, size.height as f32 / ppp);
                let hover = ui.input(|input| input.pointer.hover_pos());
                let dt = ui.input(|input| input.stable_dt);
                let pan = edge_pan(self.pan, area, dest, hover, dt);
                if pan != self.pan {
                    self.pan = pan;
                    ctx.request_repaint();
                }
            }
            let dest = image_rect(area, size, params.fit, ppp, self.pan);
            reported_geometry = Some(Geometry { rect: dest, size });
            let nearest = params.fit == Fit::Actual;
            let rendered = super::screen_view_widget_render::paint(
                self.current.as_ref(),
                &mut self.cpu,
                session,
                &painter,
                area,
                &Geometry { rect: dest, size },
                nearest,
            );
            if params.dimmed {
                painter.rect_filled(dest, 0.0, Color32::from_black_alpha(128));
            }
            shown = rendered.then_some(Geometry { rect: dest, size });
        } else if !params.status.is_empty() {
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                params.status,
                FontId::proportional(18.0),
                Color32::from_gray(210),
            );
        }
        if let Some(lines) = params.stats {
            screen_stats_overlay::show(ui, area, lines);
        }
        if let Some(hint) = params.hint.filter(|_| shown.is_some()) {
            painter.text(
                area.left_top() + Vec2::new(8.0, 8.0),
                Align2::LEFT_TOP,
                hint,
                FontId::proportional(14.0),
                Color32::from_gray(230),
            );
        }
        if let Some(report) = self.visibility.report(
            &window,
            area.intersect(ui.clip_rect()),
            reported_geometry.as_ref(),
            ppp,
        ) {
            if report.state == crate::screen::core::visibility::VisibilityState::Hidden {
                self.release_input(session);
                self.current = None;
                self.cpu.reset();
                self.source_stamp = None;
            }
            session.set_visibility(report);
        }
        let (locals, escape) = self.input(ui, session, &response, params, shown.as_ref());
        ViewResult {
            display: FrameSize::new(
                (area.width() * ppp).round().max(1.0) as u32,
                (area.height() * ppp).round().max(1.0) as u32,
            ),
            picture: size,
            locals,
            escape,
        }
    }

    /// Window input to the host while control is on; the viewer's own
    /// shortcuts and Esc are reported either way.
    fn input(
        &mut self,
        ui: &mut egui::Ui,
        session: &ViewerSession,
        response: &egui::Response,
        params: &ViewParams,
        geometry: Option<&Geometry>,
    ) -> (Vec<LocalKey>, bool) {
        let events = ui.input(|input| input.events.clone());
        let locals = local_keys(&events);
        let escape = events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        });
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(true));
        if self.was_focused && !focused {
            self.release_input(session);
        }
        self.was_focused = focused;
        let Some(hello) = session
            .info()
            .hello
            .as_ref()
            .map(|hello| hello.input.clone())
        else {
            return (locals, escape);
        };
        if !params.control || !session.input_ready() || geometry.is_none() {
            if self.mapper.is_some() {
                self.release_input(session);
            }
            return (locals, escape);
        }
        let mapper = self.mapper.get_or_insert_with(|| InputMapper::new(hello));
        // While the control is on every key belongs to the host, so the
        // picture keeps the keyboard focus (a menu that is open keeps it).
        if !ui.memory(|memory| memory.any_popup_open()) {
            response.request_focus();
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        if let (true, Some(geometry)) = (focused, geometry) {
            let modifiers = ui.input(|input| input.modifiers);
            mapper.process(&events, modifiers, geometry);
            // The IME needs the area to place its candidate window.
            ui.ctx().output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    rect: geometry.rect,
                    cursor_rect: geometry.rect,
                });
            });
            if let Some(text) = mapper.preedit.as_deref() {
                ui.painter().text(
                    geometry.rect.left_bottom() + Vec2::new(8.0, -8.0),
                    Align2::LEFT_BOTTOM,
                    text,
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
            }
        }
        if self.flush(session, false) {
            ui.ctx().request_repaint();
        }
        (locals, escape)
    }
}
