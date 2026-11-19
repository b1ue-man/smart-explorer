//! The picture area of the desktop viewer (SCR-M7, U25-5/7/8): it takes the
//! newest picture of a session, draws it (YUV planes through the GPU
//! callback, BGRA through an egui texture), keeps the statistics in a corner
//! and, while „Steuern“ is on and the window has the focus, turns the
//! window's input into `INPUT` frames. Nothing here waits for a decoder or a
//! socket: the picture comes out of a slot, frames go into a queue.
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, EventFilter, FontId, Rect, Sense, Vec2};

use super::screen_input_keys::{local_keys, LocalKey};
use super::screen_input_map::{Geometry, InputMapper};
use super::screen_prefs::Fit;
use super::screen_session::{Picture, PictureData, ViewerSession};
use super::screen_stats_overlay;
use super::screen_texture_cpu::CpuTexture;
use super::screen_wgpu_yuv::YuvFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_control::ViewerFrame;

/// What the viewer app tells the widget for one frame.
pub struct ViewParams<'a> {
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
pub struct ViewResult {
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

use super::screen_viewport::edge_pan;
pub use super::screen_viewport::image_rect;

/// The picture area of one session.
pub struct ViewWidget {
    cpu: CpuTexture,
    current: Option<Current>,
    source_stamp: Option<super::screen_presented::PresentedFrame>,
    source_geometry: Option<FrameSize>,
    pan: Vec2,
    mapper: Option<InputMapper>,
    /// The GPU path takes the planes; cleared when a picture did not fit it.
    gpu: bool,
    was_focused: bool,
    /// The picture had the keyboard last frame (`keyboard_owned`).
    keys_owned: bool,
    visibility: super::screen_visibility::VisibilityTracker,
    /// P8: Direct3D 11 display of GPU pictures (`screen_direct.rs`).
    direct: super::screen_direct::DirectView,
}

impl ViewWidget {
    pub fn new(gpu: bool) -> Self {
        Self {
            cpu: CpuTexture::default(),
            current: None,
            source_stamp: None,
            source_geometry: None,
            pan: Vec2::ZERO,
            mapper: None,
            gpu,
            was_focused: true,
            keys_owned: false,
            visibility: super::screen_visibility::VisibilityTracker::default(),
            direct: Default::default(),
        }
    }

    fn accept(&mut self, ctx: &egui::Context, session: &ViewerSession, picture: Picture) {
        self.direct.deactivate();
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
                session.recycle_bgra(image);
                self.current = Some(Current::Cpu(picture.size));
            }
            PictureData::Yuv(planes) => {
                let mut frame = YuvFrame::new(picture.size, planes);
                frame.memory = picture.memory;
                frame.pool = Some(session.picture_pool());
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
    pub fn release_input(&mut self, session: &ViewerSession) {
        if let Some(mapper) = self.mapper.as_mut() {
            mapper.release_all();
        }
        self.flush(session, true);
    }

    pub fn source_changed(&mut self, session: &ViewerSession) {
        self.release_input(session);
        session.invalidate_input();
        let _ = session.take_picture();
        // The old source's picture leaves the Direct3D 11 child too.
        self.direct.deactivate();
        self.current = None;
        self.cpu.reset();
        self.source_stamp = None;
        self.source_geometry = None;
        self.pan = Vec2::ZERO;
        self.visibility.invalidate();
    }

    pub fn retire(&mut self) {
        self.direct.deactivate();
        self.current = None;
        self.source_stamp = None;
        self.source_geometry = None;
        self.cpu.reset();
        self.visibility.invalidate();
    }
    pub fn take_texture(&mut self) -> CpuTexture {
        self.retire();
        std::mem::take(&mut self.cpu)
    }
    pub fn adopt_texture(&mut self, texture: CpuTexture) {
        self.retire();
        self.cpu = texture;
    }
    pub fn reconnect(&mut self, gpu: bool) {
        self.retire();
        self.gpu = gpu;
        self.pan = Vec2::ZERO;
        self.mapper = None;
        self.was_focused = true;
    }

    /// The mapper, made on demand (tests).
    #[cfg(test)]
    pub fn mapper_mut(
        &mut self,
        caps: crate::screen::core::input_model::InputCapabilities,
    ) -> &mut InputMapper {
        self.mapper.get_or_insert_with(|| InputMapper::new(caps))
    }

    /// Whether the shown picture came as planes (GPU) or as BGRA (BK9).
    pub fn planes_drawn(&self) -> Option<bool> {
        Some(matches!(self.current.as_ref()?, Current::Gpu(..)))
    }

    pub fn unmapped_keys(&self) -> u64 {
        self.mapper
            .as_ref()
            .map_or(0, |mapper| mapper.unmapped_keys)
    }

    /// Draws the area and handles its input.
    pub fn show(
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
        match session.take_picture() {
            Some(picture) if !window.hidden() && !session.is_ended() => {
                if session.can_present(picture.presented.clone()) {
                    self.accept(&ctx, session, picture);
                } else {
                    session.keep_if_awaiting_floor(picture);
                }
            }
            Some(_) => {}
            // The shown picture is drawn again (r1, „wiederholt gezeichnet“).
            None if self.current.is_some() && !window.hidden() => session.note_repeated(),
            None => {}
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
        // P8: the presenter's picture while it shows, else the slot's.
        let direct_size = self.direct.size().filter(|_| self.direct.showing());
        let size = direct_size
            .or_else(|| self.current.as_ref().map(Current::size))
            .or(self.source_geometry)
            .or_else(|| self.direct.size());
        let mut shown = None;
        let mut reported_geometry = None;
        if let Some(size) = size.filter(|size| size.width > 0 && size.height > 0) {
            if params.fit == Fit::Actual {
                let dest = Vec2::new(size.width as f32 / ppp, size.height as f32 / ppp);
                // Not under a toolbar or menu lying over the picture (Gemini 12).
                let hover = ui.input(|input| input.pointer.hover_pos());
                let hover = hover.filter(|_| response.contains_pointer() || response.dragged());
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
            let direct = self.direct.showing();
            let rendered = direct
                || super::screen_view_widget_render::paint(
                    self.current.as_ref(),
                    &mut self.cpu,
                    session,
                    &painter,
                    area,
                    &Geometry { rect: dest, size },
                    nearest,
                );
            if rendered && !direct {
                // The host's pointer, when it travels beside the picture (P6);
                // while this view steers it is the one pointer over the picture.
                let over = response.contains_pointer() || response.dragged();
                let steering = params.control && session.input_ready() && over;
                super::screen_cursor::paint(&painter, dest, session, steering);
            }
            if params.dimmed {
                painter.rect_filled(dest, 0.0, Color32::from_black_alpha(128));
            }
            shown = rendered.then_some(Geometry { rect: dest, size });
        }
        // Nothing drawn (no picture yet, or it was revoked after a decode
        // error): the status, else the hint, names why (fix 2026-11-19).
        let reason = Some(params.status)
            .filter(|s| !s.is_empty())
            .or(params.hint);
        if let Some(text) = reason.filter(|_| shown.is_none()) {
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(18.0),
                Color32::from_gray(210),
            );
        }
        let mut overlays: Vec<Rect> = Vec::new();
        if let Some(lines) = params.stats {
            overlays.push(screen_stats_overlay::show(ui, area, lines));
        }
        if let Some(hint) = params.hint.filter(|_| shown.is_some()) {
            let stats = overlays.first().copied();
            let field = super::screen_view_widget_render::hint(&painter, area, hint, stats);
            overlays.push(field);
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
        if self.visibility.settling() {
            ctx.request_repaint_after(super::screen_visibility::SETTLE);
        }
        let (locals, escape) = self.input(ui, session, &response, params, shown.as_ref());
        let place = super::screen_direct::DirectPlace {
            area,
            dest: reported_geometry.map(|geometry| geometry.rect),
            // Not hidden while the session does not stream (`dimmed`): the
            // child holds the last picture, egui has none to show instead.
            visible: !window.hidden() && !session.is_ended() && ui.is_rect_visible(area),
            steering: params.control,
        };
        self.direct.frame(ui, session, place, &overlays);
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
        // picture keeps the keyboard focus (a menu that is open keeps it, a
        // text field of the toolbar that is being edited too).
        let editing = super::screen_toolbar_ui::toolbar_editing(ui.ctx());
        let clicked = response.clicked() || response.drag_started();
        if clicked || (!editing && !ui.memory(|memory| memory.any_popup_open())) {
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
            let render = super::screen_view_widget_render::keyboard;
            let (keys, modifiers) = render(ui, response, mapper, &mut self.keys_owned);
            let owned = super::screen_view_widget_render::owned_events(ui, response, &events, keys);
            mapper.process(&owned, modifiers, geometry);
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
