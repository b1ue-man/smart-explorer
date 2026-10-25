//! Device-page route section. The caller supplies its polled daemon snapshot.
use crate::share::{DeviceRouteDiagnostics, ShareRouteSnapshot};

/// Call on the existing device page with the complete pinned identity. A name
/// or a device id without its public key must never select another peer's facts.
pub(crate) fn device(ui: &mut egui::Ui, routes: Option<&ShareRouteSnapshot>, device_id: &str, public_key: &str) {
    ui.label(egui::RichText::new("VERBINDUNGSWEG").strong());
    let Some(routes) = routes else {
        ui.label("Für diesen Worker ist keine Routediagnose verfügbar.");
        return;
    };
    if let Some(error) = &routes.error { ui.label(error); }
    if routes.truncated { ui.label("Routediagnose für die IPC-Größe gekürzt."); }
    let peer = routes.peers.iter().find(|peer| peer.device_id == device_id && peer.public_key == public_key);
    let Some(peer) = peer else {
        ui.label("Keine aktive Routenbeobachtung oder gespeicherte Messung für dieses Gerät.");
        return;
    };
    show_peer(ui, peer);
}

fn show_peer(ui: &mut egui::Ui, peer: &DeviceRouteDiagnostics) {
    if peer.connections.is_empty() { ui.label("Keine aktive Routenbeobachtung; vorhandene Messungen sind Verlauf."); }
    for connection in &peer.connections {
        ui.horizontal_wrapped(|ui| {
            ui.strong(&connection.label);
            ui.label(&connection.reason);
        });
        if let Some(limit) = &connection.remote_limit { ui.label(limit); }
        egui::CollapsingHeader::new(format!("Routendetails · {}", connection.protocol))
            .id_salt(("share-route", &peer.device_id, &peer.public_key, &connection.connection_id))
            .show(ui, |ui| {
                // The limit is already visible beside the state.
                for line in connection.detail_lines().into_iter().filter(|line| Some(line) != connection.remote_limit.as_ref()) { ui.label(line); }
            });
    }
    if !peer.measurements.is_empty() {
        egui::CollapsingHeader::new("Gemessener Verlauf dieses Gerätepaars")
            .id_salt(("share-route-history", &peer.device_id, &peer.public_key))
            .show(ui, |ui| {
                ui.label(crate::share::ROUTE_PAYLOAD_MEASUREMENT_NOTE);
                for sample in peer.measurements.iter().rev() { ui.label(sample.detail_line()); }
            });
    }
}
