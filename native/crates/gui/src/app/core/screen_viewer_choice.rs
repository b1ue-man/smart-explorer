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

/// A list or menu of the toolbar is open, or the pointer lies over one: the
/// full-screen toolbar stays (Codex 5/20: it closed under the pointer on its
/// way to an entry below it, and the open list went with it).
pub(super) fn menu_open(ctx: &egui::Context) -> bool {
    let over = ctx
        .pointer_hover_pos()
        .and_then(|pos| ctx.layer_id_at(pos))
        .is_some_and(|layer| layer.order != egui::Order::Background);
    over || ctx.memory(|memory| memory.any_popup_open())
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
    pub(super) fn choice(
        &mut self,
        action: &ToolbarAction,
        out: &mut Vec<Transition>,
        display: crate::screen::core::pixel_convert::FrameSize,
    ) -> bool {
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
        let Mode::View(view) = &mut self.mode else {
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
            let frame = super::super::screen_pool::decoders_frame(&self.common.backend.offers);
            let codec = self.common.prefs.codec_setting();
            let decoder = super::super::screen_prefs::decoder_choice();
            let refused = super::super::screen_session::library_refusal_for(&frame.decoders, codec)
                // Nothing left to offer (Gemini 5/7: „Hardware“ under
                // „Verlustfrei“): the daemon's text at the opening.
                .or_else(|| {
                    frame.decoders.is_empty().then(|| {
                        format!(
                            "Codec „{}“ mit Dekodierer „{}“: kein passender Dekodierer auf \
                             diesem Gerät.",
                            codec.label(),
                            decoder.label()
                        )
                    })
                });
            if let (Some(text), Some(session)) = (refused, view.session.as_ref()) {
                // No tiles in silence (N5): neither the decode thread nor the
                // host switch, the reason is shown; `true` keeps
                // `ViewState::apply` from clearing it.
                session.info().notice = Some(text);
                return true;
            }
            let local = matches!(
                action,
                ToolbarAction::Decoder(_) | ToolbarAction::DecoderLibrary(_)
            ) && view.session.as_ref().is_some_and(|s| s.switch_decoder());
            // A new codec's `SET_MODE` and `SET_DECODERS` in the order the
            // host never meets a stale `sharpness` in (Gemini 5/5): leaving
            // „Scharf (GPU)“ the mode first (a software encoder of the new
            // codec was refused as not sharp), entering it the decoders
            // first (the running codec may have no sharp encoder).
            let codec = match action {
                ToolbarAction::Codec(codec) => Some(*codec),
                _ => None,
            };
            let mode_first = codec.filter(|codec| !codec.sharpness());
            let can = view.session.as_ref().is_some_and(|s| s.can_set_decoders());
            if let Some(codec) = mode_first.filter(|_| can) {
                view.codec_mode(codec, &self.common.prefs, display);
            }
            let sent = view.session.as_ref().is_some_and(|s| s.set_decoders(frame));
            if !local && !sent {
                out.push(Transition::View(view.target.clone()));
            }
            if let Some(codec) = codec.filter(|_| sent) {
                if mode_first.is_none() {
                    view.codec_mode(codec, &self.common.prefs, display);
                }
                // The mode went out here; `ViewState::apply` would send it
                // again. What else it does for a choice: the notice goes.
                if let Some(session) = view.session.as_ref() {
                    session.info().notice = None;
                    session.shared.direct.forget_noted();
                }
                return true;
            }
        }
        false
    }
}
