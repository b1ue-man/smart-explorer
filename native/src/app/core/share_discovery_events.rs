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
}
