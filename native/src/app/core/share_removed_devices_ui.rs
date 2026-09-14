//! "Entfernte Geraete": devices the user removed. They stay listed until the
//! user readmits them or pairs them again deliberately, because the record is
//! what keeps the peer from re-installing itself automatically.
use super::*;

pub(super) fn ui(app: &mut App, ui: &mut egui::Ui) {
    let removed = app.share_profiles.removed_direct_peers.clone();
    if removed.is_empty() {
        return;
    }
    ui.separator();
    let mut readmit: Option<String> = None;
    egui::CollapsingHeader::new(format!("ENTFERNTE GERAETE ({})", removed.len()))
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                "Entfernte Geraete koppeln sich nicht mehr automatisch. Eine bewusste neue \
                 Kopplung (PIN, Direkt-Code oder Annehmen einer Anfrage) hebt die Sperre auf.",
            );
            for record in &removed {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let name = if record.device_name.trim().is_empty() {
                        record.device_id.clone()
                    } else {
                        record.device_name.clone()
                    };
                    ui.label(RichText::new(name).strong());
                    ui.label(format!("Geraet-ID: {}", record.device_id));
                    if !record.fingerprint.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Fingerprint:");
                            super::helpers::share_value_field(ui, &record.fingerprint);
                        });
                    }
                    ui.label(format!("Entfernt: {}", timestamp(record.removed_at)));
                    if ui
                        .button("Erneut zulassen")
                        .on_hover_text(
                            "Hebt nur die Sperre auf. Das Geraet kann sich danach wieder automatisch koppeln, wenn es unseren Direkt-Code noch kennt.",
                        )
                        .clicked()
                    {
                        readmit = Some(record.device_id.clone());
                    }
                });
                ui.add_space(4.0);
            }
        });
    if let Some(device_id) = readmit {
        app.readmit_removed_device(&device_id);
    }
}

fn timestamp(value: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(value, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| format!("{value} (ungueltig)"))
}
