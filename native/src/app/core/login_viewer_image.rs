//! Reuse the normal viewer's CPU texture, geometry, input map and paint proof.
use std::sync::Arc;
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use crate::login::{FrameToken, ViewerSession};
use crate::screen::core::{input_model::InputCapabilities, pixel_convert::{BgraImage, FrameSize}};
use super::super::{screen_input_map::{Geometry, InputMapper}, screen_texture_cpu::CpuTexture, screen_presented::{PresentedFrame,SessionToken}};
pub(super) struct LoginImage { texture: CpuTexture, token: Option<FrameToken>, mapper: InputMapper, was_ready: bool, rect: Option<Rect>, clip: Option<Rect>, session:std::sync::Weak<ViewerSession>, identity:SessionToken,epoch:Option<u64> }
impl Default for LoginImage {
    fn default() -> Self { Self { texture: CpuTexture::default(), token: None,
        mapper: InputMapper::new(InputCapabilities { pointer: true, keyboard: true, text: true, ..Default::default() }), was_ready: false, rect: None, clip: None, session:std::sync::Weak::new(),identity:SessionToken::default(),epoch:None } }
}
impl LoginImage {
    pub(super) fn reset(&mut self) {self.texture.reset();self.token=None;self.mapper.release_all();while self.mapper.take_batch().is_some() {}self.was_ready=false;self.rect=None;self.clip=None;self.session=std::sync::Weak::new();self.identity=SessionToken::default();self.epoch=None;}

    pub fn show(&mut self, ui: &mut egui::Ui, session: &Arc<ViewerSession>, control: bool) {
        if !self.session.upgrade().is_some_and(|old|Arc::ptr_eq(&old,session)) {self.reset();self.session=Arc::downgrade(session);}
        let snapshot = session.snapshot();
        let hidden = ui.ctx().data(|data| data.get_temp::<super::super::screen_visibility::WindowVisibility>(egui::Id::new("screen_window_visibility"))).is_some_and(|window| window.hidden());
        if hidden { session.focus(false, false); self.texture.reset(); self.token = None; return; }
        if matches!(snapshot.state.as_str(), "ended" | "stopping") { self.texture.reset(); self.token = None; session.focus(false, false); ui.label("Anmeldeverbindung beendet. Für einen neuen Kontext ausdrücklich neu starten."); return; }
        if let Some(frame) = session.take_frame() {
            let size = FrameSize::new(frame.token.width, frame.token.height);
            match BgraImage::from_vec(size, frame.pixels) {
                Ok(image) => { self.texture.update_frame(ui.ctx(), &image, stamp(session,&frame.token,self.identity.clone(),snapshot.presentation_epoch)); self.token = Some(frame.token);self.epoch=Some(snapshot.presentation_epoch); }
                Err(_) => { session.close(); return; }
            }
        }
        let height = ui.available_height().min(ui.clip_rect().height()).clamp(1.0, 480.0);
        let (area, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(1.0), height), Sense::click_and_drag());
        if !ui.is_rect_visible(area) { session.focus(false, false); self.texture.reset();self.token=None;return; }
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
        let epoch=session.snapshot().presentation_epoch;
        if self.epoch!=Some(epoch) {self.texture.reframe(stamp(session,&token,self.identity.clone(),epoch));self.epoch=Some(epoch);}
        let ppp = ui.ctx().pixels_per_point(); self.texture.prepare(ui.ctx(), [(rect.width() * ppp).round().max(1.0) as usize, (rect.height() * ppp).round().max(1.0) as usize]);
        let identity=self.identity.clone();
        self.texture.paint(&painter,rect,|stamp| {
            if !complete||stamp.session!=identity {return None;}
            let drawn=stamp.login?;let owner=session.clone();let epoch=stamp.epoch;
            Some(Arc::new(move||{owner.presented(&drawn,epoch);}) as super::super::screen_presented::Presented)
        });
        if self.texture.failed() {painter.text(rect.center(),egui::Align2::CENTER_CENTER,"Bildskalierung nicht verfügbar: Renderer- oder Speicherressource fehlt.",egui::FontId::proportional(14.0),Color32::WHITE);}
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
fn stamp(session:&Arc<ViewerSession>,token:&FrameToken,identity:SessionToken,epoch:u64)->PresentedFrame {
    let weak=Arc::downgrade(session);
    let validity=super::super::screen_render_upload::Generation::guarded(Arc::new(move||weak.upgrade().is_some_and(|owner| {
        let state=owner.snapshot();state.presentation_epoch==epoch&&!matches!(state.state.as_str(),"ended"|"stopping")
    })));
    PresentedFrame {session:identity,validity,login:Some(Arc::new(token.clone())),epoch,gop_id:1,frame_id:token.frame_id}
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::screen_render_owner::ControlledDriver;
    #[test]
    fn batches_task_b6_login_image_reset_keeps_pane_slot_until_explicit_retirement() {
        let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let pool=Arc::new(crate::screen::core::frame_memory::MemoryPool::default());
        let memory=Arc::new(pool.reserve(None,crate::screen::core::frame_budget::MAX_WORKING_BYTES).unwrap());let mut pane=LoginImage::default();
        pane.texture.bind_memory(Some(memory.clone()),Some(crate::screen::core::frame_budget::FrameLimits::LEGACY));
        pane.texture.update(&ctx,&BgraImage::new(FrameSize::new(2,2)).unwrap());pane.texture.prepare(&ctx,[2,2]);drop(memory);
        driver.upload();driver.complete_upload(pane.texture.slot());let slot=pane.texture.slot().clone();
        pane.reset();assert!(Arc::ptr_eq(&slot,pane.texture.slot()));driver.owner.after_update(&ctx);drop(pane);
        assert!(pool.reserve(None,1024*1024).is_err());driver.complete_retirement(&slot);assert!(pool.reserve(None,1024*1024).is_ok());
    }
}
