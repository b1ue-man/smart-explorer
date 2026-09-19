use super::prelude::*;
use super::*;

#[derive(Default)]
pub(super) struct ReadAccess {
    pending: Option<(String, Receiver<Result<bool, String>>)>,
    message: Option<String>,
}

impl App {
    pub(super) fn poll_read_access(&mut self) {
        let Some((root, receiver)) = &self.read_access.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("Lesehelfer wurde ohne Ergebnis beendet".into())
            }
        };
        let same_location = self.remote.is_none() && self.root_path == *root;
        self.read_access.pending = None;
        self.read_access.message = Some(match result {
            Ok(true) => {
                if same_location {
                    self.rescan();
                }
                "Leserechte verfügbar. Ergebnisse werden in dieser Ansicht aktualisiert.".into()
            }
            Ok(false) => {
                "Rechteanfrage abgebrochen. Die bisherigen Ergebnisse bleiben erhalten.".into()
            }
            Err(error) => format!(
                "Lesezugriff nicht erweitert: {error}. Die bisherigen Ergebnisse bleiben erhalten."
            ),
        });
    }

    pub(super) fn ui_read_access(&mut self, ui: &mut egui::Ui) {
        if self.progress.permission_denied > 0 {
            ui.label(format!(
                "{} Zugriff(e) verweigert",
                self.progress.permission_denied
            ));
            if self.remote.is_none() && crate::local_access::can_request_access(&self.root_path) {
                if ui.add_enabled(self.read_access.pending.is_none(),
                    egui::Button::new("Leserechte anfordern …"))
                    .on_hover_text("Geschützte Dateien nach Windows-Zustimmung in diesem Fenster lesen; die Ordnerrechte bleiben unverändert.")
                    .clicked() {
                    let root = self.root_path.clone();
                    let worker_root = root.clone();
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    match std::thread::Builder::new().name("explorer-consent".into()).spawn(move || {
                        let _ = tx.send(crate::local_access::request_access(&worker_root));
                    }) {
                        Ok(_) => self.read_access.pending = Some((root, rx)),
                        Err(error) => self.read_access.message = Some(format!("Rechteanfrage konnte nicht starten: {error}")),
                    }
                }
            } else {
                ui.label("Verbleibende Sperren: Berechtigungen oder Anmeldung am Dateisystem/Anbieter prüfen.");
            }
        }
        if self.read_access.pending.is_some() {
            ui.spinner();
            ui.label("Windows-Rechteanfrage läuft …");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
        }
        if let Some(message) = &self.read_access.message {
            ui.label(message);
        }
    }
}
