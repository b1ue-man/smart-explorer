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
        // The panel clips to the height of the pass before (egui 0.29.1
        // `TopBottomPanel::show_inside_dyn`): a row that a host answer
        // („Steuern“) or a narrower window adds lay outside it for one pass.
        // A changed height discards that pass and lays it out again
        // (Codex 12/16).
        let shown = egui::TopBottomPanel::top("screen_toolbar").show(ctx, add);
        let id = egui::Id::new("screen_toolbar_height");
        let height = shown.response.rect.height();
        let before = ctx.data_mut(|data| data.get_temp::<f32>(id));
        ctx.data_mut(|data| data.insert_temp(id, height));
        if before.is_some_and(|before| (before - height).abs() > 0.5) {
            ctx.request_discard("screen toolbar changed its height");
        }
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
        let prefs = super::super::screen_prefs::ViewerPrefs::clone(&self.common.prefs);
        let before = (
            super::super::screen_prefs::codec_setting(),
            super::super::screen_prefs::decoder_choice(),
            super::super::screen_prefs::decoder_library_choice(),
        );
        let again = screen_toolbar_ui::apply_codec(action, &mut self.common.prefs);
        let Mode::View(view) = &mut self.mode else {
            return false;
        };
        let refused = (again && named.is_some())
            .then(|| {
                refusal(
                    &self.common.backend.offers,
                    self.common.prefs.codec_setting(),
                )
            })
            .flatten();
        if let (Some(text), Some(session)) = (refused, view.session.as_ref()) {
            // An impossible pair is neither stored nor taken over by the
            // decode thread or the host (Gemini 8/7: it stayed in
            // `viewer.json` and in the process's decoder choice); the reason
            // is shown, `true` keeps `ViewState::apply` from clearing it.
            self.common.prefs = prefs;
            use super::super::screen_prefs::{set_codec_choice, set_output_choice};
            let restored = match action {
                ToolbarAction::Codec(_) => set_codec_choice(Some(before.0), None),
                ToolbarAction::Decoder(_) => set_codec_choice(None, Some(before.1)),
                ToolbarAction::DecoderLibrary(_) => set_output_choice(Some(before.2), None),
                _ => Ok(()),
            };
            super::super::screen_toolbar_ui::note_save(restored);
            session.info().notice = Some(text);
            return true;
        }
        // A session that began with the tiles takes no video codec by
        // `SET_DECODERS`: it opens again with the choice.
        if let Some(codec) = named {
            if view.session.as_ref().is_some_and(|s| s.needs_reopen(codec)) {
                out.push(Transition::View(view.target.clone()));
                return true;
            }
        }
        if again {
            let request = super::super::screen_pool::announced_request(&self.common.backend.offers);
            let frame = request.decoders_frame();
            // Codex 8/7: the decode thread's codec watch starts afresh.
            if let Some(session) = view.session.as_ref() {
                session.choice_made();
            }
            let local = matches!(
                action,
                ToolbarAction::Decoder(_) | ToolbarAction::DecoderLibrary(_)
            ) && view.session.as_ref().is_some_and(|s| s.switch_decoder());
            // A new codec: `SET_DECODERS` first, its `SET_MODE` after. The
            // host reads the sharpness of the fallback range from
            // `codec_choice` (round 8), so one encoder starts, not two when
            // „Scharf (GPU)“ is left (Gemini 5/5).
            let codec = match action {
                ToolbarAction::Codec(codec) => Some(*codec),
                _ => None,
            };
            let sent = view.session.as_ref().is_some_and(|s| s.set_decoders(frame));
            if !local && !sent {
                out.push(Transition::View(view.target.clone()));
            }
            if let Some(codec) = codec.filter(|_| sent) {
                view.codec_mode(codec, &self.common.prefs, display);
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

/// Why the stored „Codec“, „Dekodierer“ and „Bibliothek“ cannot work here:
/// the library looks at the offers before „Codec“/„Dekodierer“ narrow them,
/// as at the opening (round 8 B3); nothing left to offer names the pair
/// (Gemini 5/7: „Hardware“ under „Verlustfrei“).
fn refusal(
    offers: &[crate::screen::core::negotiate::CodecOffer],
    codec: crate::screen::core::negotiate::CodecSetting,
) -> Option<String> {
    let request = super::super::screen_pool::announced_request(offers);
    let library = super::super::screen_session::library_refusal_for(&request.decoders, codec);
    library.or_else(|| {
        let decoder = super::super::screen_prefs::decoder_choice();
        request.decoders_frame().decoders.is_empty().then(|| {
            format!(
                "Codec „{}“ mit Dekodierer „{}“: kein passender Dekodierer auf diesem Gerät.",
                codec.label(),
                decoder.label()
            )
        })
    })
}
