//! Pure registry rendering: no storage or network call on the UI thread.
use crate::app::prelude::egui;

pub(super) fn render(ui: &mut egui::Ui) {
    let states = crate::source_media::snapshots();
    if states.is_empty() {
        return;
    }
    ui.collapsing("Optische Leser dieses Prozesses", |ui| {
        for state in states {
            let mode = if !state.active {
                "wartet auf Laufwerk"
            } else {
                match state.mode {
                    crate::source_media::ReadMode::Waiting => "wartet auf Laufwerk",
                    crate::source_media::ReadMode::Exclusive => "exklusiver Lesezugriff",
                    crate::source_media::ReadMode::Sequential => "liest sequenziell",
                    crate::source_media::ReadMode::MetadataFallback => {
                        "liest mit bisherigem Metadaten-Kopierweg"
                    }
                }
            };
            ui.label(format!("{} · {mode}", state.source.device));
            if let Some(bytes) = state.block_bytes {
                ui.small(format!("Leseblock: {bytes} Bytes"));
            }
            ui.small(format!(
                "Wartende Aufträge dieses Prozesses: {} ausdrücklich, {} Hintergrund",
                state.foreground_waiters, state.background_waiters
            ));
        }
        ui.small("Die Anzeige ordnet diese Leser keinem einzelnen Kopierauftrag zu.");
    });
}
