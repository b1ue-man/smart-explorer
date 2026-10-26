//! Reuse the normal viewer's CPU texture, geometry, input map and paint proof.
use std::sync::Arc;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use crate::login::{FrameToken, ViewerSession};
use crate::screen::core::{input_model::InputCapabilities, pixel_convert::{BgraImage, FrameSize}};
use super::super::{screen_input_map::{Geometry, InputMapper}, screen_texture_cpu::CpuTexture, screen_presented::{self, PresentedFrame}};
pub(super) struct LoginImage { texture: CpuTexture, token: Option<FrameToken>, mapper: InputMapper, was_ready: bool, rect: Option<Rect>, clip: Option<Rect> }
impl Default for LoginImage {
    fn default() -> Self { Self { texture: CpuTexture::default(), token: None,
        mapper: InputMapper::new(InputCapabilities { pointer: true, keyboard: true, text: true, ..Default::default() }), was_ready: false, rect: None, clip: None } }
}
impl LoginImage {
    pub fn show(&mut self, ui: &mut egui::Ui, session: &Arc<ViewerSession>, control: bool) {
        let snapshot = session.snapshot();
        let hidden = ui.ctx().data(|data| data.get_temp::<super::super::screen_visibility::WindowVisibility>(egui::Id::new("screen_window_visibility"))).is_some_and(|window| window.hidden());
        if hidden { session.focus(false, false); self.texture = CpuTexture::default(); self.token = None; return; }
        if matches!(snapshot.state.as_str(), "ended" | "stopping") { self.texture = CpuTexture::default(); self.token = None; session.focus(false, false); ui.label("Anmeldeverbindung beendet. Für einen neuen Kontext ausdrücklich neu starten."); return; }
        if let Some(frame) = session.take_frame() {
            let size = FrameSize::new(frame.token.width, frame.token.height);
            match BgraImage::from_vec(size, frame.pixels) {
                Ok(image) => { self.texture.update_frame(ui.ctx(), &image, PresentedFrame { epoch: 1, gop_id: 1, frame_id: frame.token.frame_id }); self.token = Some(frame.token); }
                Err(_) => { session.close(); return; }
            }
        }
        let height = ui.available_height().min(ui.clip_rect().height()).clamp(1.0, 480.0);
        let (area, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(1.0), height), Sense::click_and_drag());
        if !ui.is_rect_visible(area) { session.focus(false, false); return; }
        session.visible();
        let painter = ui.painter_at(area); painter.rect_filled(area, 0.0, Color32::BLACK);
        if response.clicked() { response.request_focus(); }
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(false)) && response.has_focus();
        let Some(token) = self.token.clone() else { session.focus(false, false); return; };
        let size = FrameSize::new(token.width, token.height);
        let scale = (area.width() / token.width as f32).min(area.height() / token.height as f32);
        let rect = Rect::from_center_size(area.center(), Vec2::new(token.width as f32 * scale, token.height as f32 * scale));
        let clip = rect.intersect(ui.clip_rect());
        if self.rect.is_some_and(|old| old != rect) || self.clip.is_some_and(|old| old != clip) { session.invalidate_presentation(); }
        self.rect = Some(rect); self.clip = Some(clip);
        let complete = ui.clip_rect().contains_rect(rect);
        session.focus(focused && complete, control && complete);
        let ppp = ui.ctx().pixels_per_point(); self.texture.prepare(ui.ctx(), [(rect.width() * ppp).round().max(1.0) as usize, (rect.height() * ppp).round().max(1.0) as usize]);
        if let Some(texture) = self.texture.id() {
            painter.image(texture, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            if complete && !self.texture.failed() && self.texture.presented_frame().is_some_and(|presented| presented.frame_id == token.frame_id) {
                let owner = session.clone(); let drawn = token.clone(); let epoch = session.snapshot().presentation_epoch;
                painter.add(screen_presented::cpu_callback(rect, Arc::new(move || { owner.presented(&drawn, epoch); }),
                    ui.ctx().data(|data| data.get_temp::<bool>(egui::Id::new("screen_renderer_wgpu"))).unwrap_or(true)));
            }
        }
        let ready = complete && session.snapshot().input_ready && control && focused;
        if self.was_ready && !ready { self.mapper.release_all(); while self.mapper.take_batch().is_some() {} session.release(); session.focus(focused, control); }
        self.was_ready = ready;
        if ready {
            let (events, modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
            self.mapper.process(&events, modifiers, &Geometry { rect, size });
            if let Some(events) = self.mapper.take_batch() {
                let state = session.snapshot();
                if let Some(shown) = state.presented_frame { if session.input(shown, state.presentation_epoch, events).is_err() { self.mapper.release_all(); session.release(); } }
            }
        }
    }
}
