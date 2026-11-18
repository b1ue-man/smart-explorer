//! „Codec“, „Dekodierer“, „Dekodierer-Bibliothek“ and „Anzeige“ chosen in
//! the toolbar of the full view, carried out (user rule of 2026-11-18:
//! every choice is used exactly as chosen).
use super::super::screen_toolbar_ui::{self, ToolbarAction};
use super::{Mode, ScreenViewerApp, Transition};

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
        let named = match action {
            ToolbarAction::Codec(codec) => Some(*codec),
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
