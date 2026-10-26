//! Reuse the normal viewer's CPU texture, geometry, input map and paint proof.
use super::super::{
    screen_input_map::{Geometry, InputMapper},
    screen_presented::{PresentedFrame, SessionToken},
    screen_texture_cpu::CpuTexture,
};
use crate::login::{FrameToken, ViewerSession};
use crate::screen::core::{input_model::InputCapabilities, pixel_convert::FrameSize};
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
pub(super) struct LoginImage {
    texture: CpuTexture,
    token: Option<FrameToken>,
    mapper: InputMapper,
    was_ready: bool,
    rect: Option<Rect>,
    clip: Option<Rect>,
    session: std::sync::Weak<ViewerSession>,
    identity: SessionToken,
    epoch: Option<u64>,
    window_active: Arc<AtomicBool>,
}
impl Default for LoginImage {
    fn default() -> Self {
        Self {
            texture: CpuTexture::default(),
            token: None,
            mapper: InputMapper::new(InputCapabilities {
                pointer: true,
                keyboard: true,
                text: true,
                ..Default::default()
            }),
            was_ready: false,
            rect: None,
            clip: None,
            session: std::sync::Weak::new(),
            identity: SessionToken::default(),
            epoch: None,
            window_active: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl LoginImage {
    pub(super) fn reset(&mut self) {
        self.window_active.store(false, Ordering::Release);
        self.window_active = Arc::new(AtomicBool::new(false));
        self.texture.reset();
        self.token = None;
        self.mapper.release_all();
        while self.mapper.take_batch().is_some() {}
        self.was_ready = false;
        self.rect = None;
        self.clip = None;
        self.session = std::sync::Weak::new();
        self.identity = SessionToken::default();
        self.epoch = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, session: &Arc<ViewerSession>, control: bool) {
        if !self
            .session
            .upgrade()
            .is_some_and(|old| Arc::ptr_eq(&old, session))
        {
            self.reset();
            self.session = Arc::downgrade(session);
        }
        let snapshot = session.snapshot();
        let hidden = ui
            .ctx()
            .data(|data| {
                data.get_temp::<super::super::screen_visibility::WindowVisibility>(egui::Id::new(
                    "screen_window_visibility",
                ))
            })
            .is_some_and(|window| window.hidden());
        if hidden {
            self.window_active.store(false, Ordering::Release);
            session.focus(false, false);
            self.texture.reset();
            self.token = None;
            return;
        }
        if matches!(snapshot.state.as_str(), "ended" | "stopping") {
            self.window_active.store(false, Ordering::Release);
            self.texture.reset();
            self.token = None;
            session.focus(false, false);
            ui.label(
                "Anmeldeverbindung beendet. Für einen neuen Kontext ausdrücklich neu starten.",
            );
            return;
        }
        if let Some(frame) = session.take_frame() {
            if frame.image.size() != FrameSize::new(frame.token.width, frame.token.height) {
                session.close();
                return;
            }
            if self.texture.update_local_frame(
                ui.ctx(),
                &frame.image,
                stamp(
                    session,
                    &frame.token,
                    self.identity.clone(),
                    snapshot.presentation_epoch,
                ),
            ) {
                self.token = Some(frame.token);
                self.epoch = Some(snapshot.presentation_epoch);
            }
        }
        let height = ui
            .available_height()
            .min(ui.clip_rect().height())
            .clamp(1.0, 480.0);
        let (area, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(1.0), height),
            Sense::click_and_drag(),
        );
        if !ui.is_rect_visible(area) {
            self.window_active.store(false, Ordering::Release);
            session.focus(false, false);
            self.texture.reset();
            self.token = None;
            return;
        }
        session.visible();
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, Color32::BLACK);
        if response.clicked() {
            response.request_focus();
        }
        let window_focused = ui.input(|input| input.viewport().focused.unwrap_or(false));
        if self.window_active.swap(window_focused, Ordering::AcqRel) != window_focused {
            session.invalidate_presentation();
        }
        let focused = window_focused && response.has_focus();
        let Some(token) = self.token.clone() else {
            session.focus(false, false);
            return;
        };
        let size = FrameSize::new(token.width, token.height);
        let scale = (area.width() / token.width as f32).min(area.height() / token.height as f32);
        let rect = Rect::from_center_size(
            area.center(),
            Vec2::new(token.width as f32 * scale, token.height as f32 * scale),
        );
        let clip = rect.intersect(ui.clip_rect());
        if self.rect.is_some_and(|old| old != rect) || self.clip.is_some_and(|old| old != clip) {
            session.invalidate_presentation();
        }
        self.rect = Some(rect);
        self.clip = Some(clip);
        let complete = ui.clip_rect().contains_rect(rect);
        session.focus(focused && complete, control && complete);
        let epoch = session.snapshot().presentation_epoch;
        if self.epoch != Some(epoch) {
            self.texture
                .reframe(stamp(session, &token, self.identity.clone(), epoch));
            self.epoch = Some(epoch);
        }
        let ppp = ui.ctx().pixels_per_point();
        self.texture.prepare(
            ui.ctx(),
            [
                (rect.width() * ppp).round().max(1.0) as usize,
                (rect.height() * ppp).round().max(1.0) as usize,
            ],
        );
        let identity = self.identity.clone();
        let active = self.window_active.clone();
        self.texture.paint(&painter, rect, |stamp| {
            if !complete || !window_focused || stamp.session != identity {
                return None;
            }
            let drawn = stamp.login?;
            let owner = session.clone();
            let epoch = stamp.epoch;
            let active = active.clone();
            Some(Arc::new(move || {
                if active.load(Ordering::Acquire) {
                    owner.presented(&drawn, epoch);
                }
            }) as super::super::screen_presented::Presented)
        });
        if self.texture.failed() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Bildskalierung nicht verfügbar: Renderer- oder Speicherressource fehlt.",
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        let ready = complete && session.snapshot().input_ready && control && focused;
        if self.was_ready && !ready {
            self.mapper.release_all();
            while self.mapper.take_batch().is_some() {}
            session.release();
            session.focus(focused, control);
        }
        self.was_ready = ready;
        if ready {
            let (events, modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
            self.mapper
                .process(&events, modifiers, &Geometry { rect, size });
            if let Some(events) = self.mapper.take_batch() {
                let state = session.snapshot();
                if let Some(shown) = state.presented_frame {
                    if session
                        .input(shown, state.presentation_epoch, events)
                        .is_err()
                    {
                        self.mapper.release_all();
                        session.release();
                    }
                }
            }
        }
    }
}
fn stamp(
    session: &Arc<ViewerSession>,
    token: &FrameToken,
    identity: SessionToken,
    epoch: u64,
) -> PresentedFrame {
    let weak = Arc::downgrade(session);
    let failed = weak.clone();
    let validity = super::super::screen_render_upload::Generation::revocable(
        Arc::new(move || {
            weak.upgrade().is_some_and(|owner| {
                let state = owner.snapshot();
                state.presentation_epoch == epoch
                    && !matches!(state.state.as_str(), "ended" | "stopping")
            })
        }),
        Arc::new(move || {
            if let Some(owner) = failed.upgrade() {
                owner.renderer_failed(epoch);
            }
        }),
    );
    PresentedFrame {
        session: identity,
        validity,
        login: Some(Arc::new(token.clone())),
        epoch,
        gop_id: 1,
        frame_id: token.frame_id,
    }
}

#[cfg(test)]
#[path = "batches_renderer_failure_task_tests.rs"]
mod renderer_failure_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::screen_render_owner::ControlledDriver;
    use crate::screen::core::image_admission::ImageRole;
    #[test]
    fn batches_task_b6_login_image_real_ipc_show_keeps_local_claims_until_render_retirement() {
        let fixture = crate::daemon::login_image_fixture(FrameSize::new(8, 8)).unwrap();
        assert!(fixture.wait_frame(std::time::Duration::from_secs(3)));
        let memory =
            fixture.image_admission(crate::app::screen_image_memory::owner_limits().unwrap());
        let ctx = egui::Context::default();
        let driver = ControlledDriver::install(&ctx);
        let mut pane = LoginImage::default();
        pane.texture.set_test_local(memory.clone());
        let show = |pane: &mut LoginImage, focused: bool| {
            let mut raw = egui::RawInput {
                focused,
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(8.0))),
                ..Default::default()
            };
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .focused = Some(focused);
            ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| pane.show(ui, &fixture.session, false));
            })
        };
        drop(show(&mut pane, true));
        driver.upload();
        driver.complete_upload(pane.texture.slot());
        assert_eq!(memory.live_count(ImageRole::RenderSurface).unwrap(), 1);
        assert_eq!(memory.live_count(ImageRole::UploadStaging).unwrap(), 0);
        let output = show(&mut pane, true);
        assert!(
            fixture.session.snapshot().presented_frame.is_none(),
            "UI preparation is not a draw receipt"
        );
        assert_eq!(driver.draw_output(&output), 1);
        assert!(fixture.session.snapshot().presented_frame.is_some());
        let stale = show(&mut pane, true);
        let unfocused = show(&mut pane, false);
        assert!(fixture.session.snapshot().presented_frame.is_none());
        driver.draw_output(&stale);
        driver.draw_output(&unfocused);
        assert!(
            fixture.session.snapshot().presented_frame.is_none(),
            "neither old nor new unfocused draw can authorize SAS"
        );
        drop(show(&mut pane, true));
        driver.owner.after_update(&ctx);
        driver.complete_retirement(pane.texture.slot());
        driver.upload();
        driver.complete_upload(pane.texture.slot());
        let refocused = show(&mut pane, true);
        driver.draw_output(&refocused);
        assert!(
            fixture.session.snapshot().presented_frame.is_some(),
            "window focus is sufficient without stealing widget focus from SAS"
        );
        drop(stale);
        drop(unfocused);
        drop(refocused);
        let slot = pane.texture.slot().clone();
        drop(output);
        fixture.session.close();
        pane.reset();
        drop(pane);
        driver.owner.after_update(&ctx);
        fixture.finish().unwrap();
        assert_eq!(memory.live_count(ImageRole::RenderSurface).unwrap(), 1);
        driver.complete_retirement(&slot);
        assert_eq!(memory.live_bytes().unwrap(), 0);
    }
}
