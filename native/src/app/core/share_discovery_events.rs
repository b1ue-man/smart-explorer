//! Discovery command dispatch and event folding live in
//! `crate::share::discovery_events`; the GUI hands in a repaint of its context.
use super::share_discovery_state::DiscoveryUiAction;
use super::App;
use crate::share::discovery_events;
use eframe::egui;

impl App {
    pub(in crate::app) fn drain_discovery_command_results(&mut self) {
        discovery_events::drain_discovery_command_results(&mut self.share_discovery);
    }

    pub(in crate::app) fn dispatch_discovery_ui_action(
        &mut self,
        action: DiscoveryUiAction,
        repaint: egui::Context,
    ) {
        if let DiscoveryUiAction::Revoke { exchange_id } = &action {
            self.revoke_unconfirmed_pairing(exchange_id);
            return;
        }
        discovery_events::dispatch_discovery_ui_action(
            &mut self.share_discovery,
            action,
            Box::new(move || repaint.request_repaint()),
        );
    }

    pub(in crate::app) fn apply_share_discovery_event(
        &mut self,
        event: crate::share::DiscoveryEvent,
    ) {
        discovery_events::apply_share_discovery_event(&mut self.share_discovery, event);
    }

    /// "Widerrufen" of a pairing that lacks confirmation (S24): removes the
    /// installed contact or room like "Entfernen" does.
    fn revoke_unconfirmed_pairing(&mut self, exchange_id: &str) {
        use crate::share::DiscoveryRelationOutcome as Outcome;
        match self.share_discovery.take_unconfirmed(exchange_id) {
            Some(Outcome::DirectInstalled { contact_id, .. }) => {
                self.remove_direct_peer_completely(&contact_id);
            }
            Some(Outcome::RoomInstalled {
                room_profile_id, ..
            }) => self.remove_room_completely(&room_profile_id),
            Some(Outcome::RoomShared { .. }) | None => {}
        }
    }
}
