//! „Codec“, „Dekodierer“, „Dekodierer-Bibliothek“ and „Anzeige“ chosen in
//! the toolbar of the full view, carried out (user rule of 2026-11-18:
//! every choice is used exactly as chosen).
use super::super::screen_toolbar_ui::{self, ToolbarAction};
use super::{Mode, ScreenViewerApp, Transition};
use eframe::egui;

/// The toolbar: a panel above the picture in a window; in full screen it
/// lies over the picture (fix 2026-11-19), in a layer of
/// `Order::Foreground` the Direct3D 11 child cuts out like a popup. As a
/// panel each time it showed or hid in full screen it resized the picture
/// area: a visibility report (1:1) and maybe a new size, so a new encoder.
pub(super) fn toolbar(ctx: &egui::Context, fullscreen: bool, add: impl FnOnce(&mut egui::Ui)) {
    if !fullscreen {
        egui::TopBottomPanel::top("screen_toolbar").show(ctx, add);
        return;
    }
    let width = ctx.screen_rect().width();
    let shown = egui::Area::new(egui::Id::new("screen_toolbar_overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::Pos2::ZERO)
        .show(ctx, |ui| {
            let frame = egui::Frame::side_top_panel(ui.style());
            let inner = width - frame.total_margin().sum().x;
            frame.show(ui, |ui| {
                ui.set_width(inner.max(0.0));
                add(ui);
            });
        });
    // The hint over the picture goes below it (`toolbar_overlay_bottom`).
    let bottom = (ctx.cumulative_pass_nr(), shown.response.rect.bottom());
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(OVERLAY_BOTTOM), bottom));
}

const OVERLAY_BOTTOM: &str = "screen_toolbar_overlay_bottom";

/// Where the full-screen toolbar drawn in this pass ends (points), if any.
pub fn toolbar_overlay_bottom(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|data| data.get_temp::<(u64, f32)>(egui::Id::new(OVERLAY_BOTTOM)))
        .filter(|(pass, _)| *pass == ctx.cumulative_pass_nr())
        .map(|(_, bottom)| bottom)
}

impl ScreenViewerApp {
    /// Stores the choice and changes the session: a decoder that decodes
    /// the running stream takes over at the next key frame (V1), the host
    /// negotiates again (`SET_DECODERS`, P1), or the session opens again.
    /// `true` when the session opens again before the view applies the
    /// choice to the running one.
    pub(super) fn choice(&mut self, action: &ToolbarAction, out: &mut Vec<Transition>) -> bool {
        // A codec given up under „Automatisch“ is out of its session only
        // (fix of 2026-11-18); the new choice takes the session's give-ups
        // back (`ViewerSession::codec_chosen`).
        // A „Dekodierer“ or „Dekodier-Bibliothek“ asks the same of a session
        // that began with the tiles (Gemini 8: `SET_DECODERS` cannot take it
        // to a video codec, so the choice stayed without effect).
        let named = match action {
            ToolbarAction::Codec(codec) => Some(*codec),
            ToolbarAction::Decoder(_) | ToolbarAction::DecoderLibrary(_) => {
                Some(self.common.prefs.codec_setting())
            }
            _ => None,
        };
        let again = screen_toolbar_ui::apply_codec(action, &mut self.common.prefs);
        let Mode::View(view) = &self.mode else {
            return false;
        };
        // A session that began with the tiles takes no video codec by
        // `SET_DECODERS`: it opens again with the choice.
        if let Some(codec) = named {
            if view.session.as_ref().is_some_and(|s| s.needs_reopen(codec)) {
                out.push(Transition::View(view.target.clone()));
                return true;
            }
        }
        if again {
            let local = matches!(
                action,
                ToolbarAction::Decoder(_) | ToolbarAction::DecoderLibrary(_)
            ) && view.session.as_ref().is_some_and(|s| s.switch_decoder());
            let frame = super::super::screen_pool::decoders_frame(&self.common.backend.offers);
            let sent = view.session.as_ref().is_some_and(|s| s.set_decoders(frame));
            if !local && !sent {
                out.push(Transition::View(view.target.clone()));
            }
        }
        false
    }
}
