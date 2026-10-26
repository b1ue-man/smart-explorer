//! Renderer for P3 content. The owner fetches IconRequest through its authenticated target.
use crate::screen::core::app_icon::{
    decode_png, IconRef, IconReply, IconRequest, IconResponse, IconTarget,
};
use crate::screen::core::icon_cache::{IconCache, IconCacheKey};
use eframe::egui;
use std::collections::HashMap;
use std::io;

#[derive(Default)]
pub(crate) struct ScreenIcons {
    content: IconCache,
    textures: HashMap<IconCacheKey, egui::TextureHandle>,
    pub(crate) requests: super::screen_icon_requests::IconRequests,
}

impl std::fmt::Debug for ScreenIcons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenIcons")
            .field("textures", &self.textures.len())
            .finish_non_exhaustive()
    }
}

impl ScreenIcons {
    pub(crate) fn receive(&mut self, peer: &str, response: &IconResponse) {
        if self.requests.receive(response) {
            if let Some(reply) = &response.content {
                let _ = self.remember(peer, reply);
            }
        }
    }
    pub(crate) fn show_target(
        &mut self,
        ui: &mut egui::Ui,
        peer: &str,
        target: IconTarget,
        icon: Option<&IconRef>,
        side: f32,
    ) -> egui::Response {
        if let Some(icon) = icon.filter(|icon| !self.contains(peer, icon)) {
            self.requests.request(IconRequest {
                target,
                icon: icon.clone(),
            });
        }
        self.show(ui, peer, icon, side)
    }
    pub(crate) fn remember(&mut self, peer: &str, reply: &IconReply) -> io::Result<()> {
        let key = IconCacheKey {
            peer: peer.into(),
            icon: reply.icon.clone(),
        };
        self.content.insert(key, reply)?;
        self.textures.retain(|stored, _| {
            stored.peer != peer
                || stored.icon.app_id != reply.icon.app_id
                || stored.icon == reply.icon
        });
        Ok(())
    }

    pub(crate) fn contains(&self, peer: &str, icon: &IconRef) -> bool {
        self.content
            .get(&IconCacheKey {
                peer: peer.into(),
                icon: icon.clone(),
            })
            .is_some()
    }

    pub(crate) fn forget_peer(&mut self, peer: &str) {
        self.content.forget_peer(peer);
        self.textures.retain(|key, _| key.peer != peer);
    }

    pub(crate) fn texture(
        &mut self,
        ctx: &egui::Context,
        peer: &str,
        icon: &IconRef,
    ) -> Option<egui::TextureId> {
        let key = IconCacheKey {
            peer: peer.into(),
            icon: icon.clone(),
        };
        if let Some(texture) = self.textures.get(&key) {
            return Some(texture.id());
        }
        let bytes = self.content.get(&key)?;
        let (width, height, rgba) = decode_png(bytes).ok()?;
        // Bound GPU retention separately from compressed-byte retention (16 MiB maximum).
        if self.textures.len() >= 64 {
            self.textures.clear();
        }
        let image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let texture = ctx.load_texture(
            format!("screen-icon:{}:{}", peer, icon.sha256),
            image,
            egui::TextureOptions::LINEAR,
        );
        let id = texture.id();
        self.textures.insert(key, texture);
        Some(id)
    }

    /// A neutral window symbol is visible while unavailable, loading or rejected.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        peer: &str,
        icon: Option<&IconRef>,
        side: f32,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        if let Some(texture) = icon.and_then(|icon| self.texture(ui.ctx(), peer, icon)) {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            let neutral = rect.shrink(side * 0.15);
            let stroke = ui.visuals().widgets.noninteractive.fg_stroke;
            ui.painter().rect_stroke(neutral, side * 0.06, stroke);
            let y = neutral.top() + side * 0.2;
            ui.painter().line_segment(
                [
                    egui::pos2(neutral.left(), y),
                    egui::pos2(neutral.right(), y),
                ],
                stroke,
            );
        }
        response
    }
}
