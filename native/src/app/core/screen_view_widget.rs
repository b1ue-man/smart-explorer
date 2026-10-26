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

enum Current {
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
            pan: Vec2::ZERO,
            mapper: None,
            gpu,
            was_focused: true,
            visibility: super::screen_visibility::VisibilityTracker::default(),
        }
    }

    fn accept(&mut self, ctx: &egui::Context, session: &ViewerSession, picture: Picture) {
        match picture.data {
            PictureData::Bgra(image) => {
                self.cpu.update_frame(ctx, &image, picture.presented);
                self.current = Some(Current::Cpu(picture.size));
            }
            PictureData::Yuv(planes) => {
                let frame = YuvFrame::new(picture.size, planes);
                let fits_texture = super::screen_wgpu_yuv::texture_limit().is_some_and(|limit|
                    frame.size.width <= limit && frame.size.height <= limit);
                if self.gpu && frame.fits() && fits_texture {
                    self.current = Some(Current::Gpu(Arc::new(frame), picture.presented));
                } else {
                    // The next pictures come as BGRA.
                    self.gpu = false;
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
        self.cpu = CpuTexture::default();
        self.pan = Vec2::ZERO;
        self.visibility.invalidate();
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
        let window = ctx.data(|data| data.get_temp::<super::screen_visibility::WindowVisibility>(egui::Id::new("screen_window_visibility"))).unwrap_or_default();
        if window.hidden() {
            self.release_input(session);
            self.current = None;
            self.cpu = CpuTexture::default();
        }
        session.refresh_visibility();
        if let Some(picture) = session.take_picture() {
            if !window.hidden() && session.can_present(picture.presented) {
                self.accept(&ctx, session, picture);
            }
        }
        let ppp = ctx.pixels_per_point();
        let available = ui.available_size();
        let (area, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, Color32::BLACK);
        let size = self.current.as_ref().map(Current::size);
        let mut shown = None;
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
            let nearest = params.fit == Fit::Actual;
            let mut rendered = false;
            match self.current.as_ref() {
                Some(Current::Cpu(_)) => {
                    self.cpu.prepare(&ctx, if nearest { [size.width as usize, size.height as usize] }
                        else { [(dest.width()*ppp).round().max(1.0) as usize, (dest.height()*ppp).round().max(1.0) as usize] });
                    if self.cpu.failed() {
                        painter.text(area.center(),Align2::CENTER_CENTER,"Bildskalierung nicht verfügbar: Renderer- oder Speicherressource fehlt.",FontId::proportional(16.0),Color32::WHITE);
                    }
                    if let Some(texture) = self.cpu.id() {
                        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                        painter.image(texture, dest, uv, Color32::WHITE);
                        rendered = !self.cpu.failed();
                        if rendered {
                            if let Some(frame) = self.cpu.presented_frame().filter(|frame| session.can_present(*frame)) {
                                painter.add(super::screen_presented::cpu_callback(dest,session.presented_callback(frame),
                                    ctx.data(|data| data.get_temp::<bool>(egui::Id::new("screen_renderer_wgpu"))).unwrap_or(true)));
                            }
                        }
                    }
                }
                Some(Current::Gpu(frame, presented)) => {
                    painter.add(super::screen_wgpu_yuv::paint_presented(dest, frame.clone(), nearest,
                        session.presented_callback(*presented)));
                    rendered = true;
                }
                None => {}
            }
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
        if let Some(report) = self.visibility.report(&window, area.intersect(ui.clip_rect()), shown.as_ref(), ppp) {
            if report.state == crate::screen::core::visibility::VisibilityState::Hidden {
                self.release_input(session);
                self.current = None;
                self.cpu = CpuTexture::default();
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
