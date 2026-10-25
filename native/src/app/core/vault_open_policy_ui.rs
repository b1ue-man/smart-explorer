//! Identity-bound opening permissions; independent from mount preferences.
use crossbeam_channel::Receiver;
use eframe::egui;
use super::vault_feed::{poke, poll, spawn, Poll};

struct Snapshot {
    stored: crate::vault::OpenPolicy,
    current: Result<crate::vault::MountPolicy, String>,
}

fn snapshot(site: &str) -> Result<Snapshot, String> {
    let record = crate::vault::RegistryStore::open_default().resolve(site)?;
    Ok(Snapshot {
        stored: crate::vault::stored_policy(&record)?,
        current: crate::vault::mount_policy(&record.id),
    })
}

#[derive(Default)]
pub(super) struct OpenPolicyInfo {
    value: Option<Snapshot>,
    rx: Option<Receiver<Result<Snapshot, String>>>,
    error: Option<String>,
    loaded: bool,
}

impl OpenPolicyInfo {
    fn start(&mut self, ctx: &egui::Context, site: &str, change: impl FnOnce(&str) -> Result<(), String> + Send + 'static) {
        let site = site.to_string();
        self.error = None;
        match spawn(ctx, "vault-open-policy", move || {
            change(&site)?;
            snapshot(&site)
        }) {
            Ok(rx) => { self.rx = Some(rx); self.loaded = true; }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, site: &str) {
        match poll(&mut self.rx) {
            Poll::Ready(Ok(value)) => { self.value = Some(value); self.error = None; poke(); }
            Poll::Ready(Err(error)) => { self.error = Some(error); self.value = None; }
            Poll::Gone => self.error = Some("Die Richtlinienabfrage endete ohne Antwort.".into()),
            Poll::Pending => {}
        }
        if !self.loaded { self.start(ui.ctx(), site, |_| Ok(())); }
        let busy = self.rx.is_some();
        ui.label(egui::RichText::new("Speichervertrauen").strong());
        let mut grant = None;
        let mut revoke_root = false;
        let mut revoke_copy = false;
        if let Some(value) = &self.value {
            match &value.current {
                Ok(policy) => {
                    ui.label(if policy.enforced { "Technische Rootbegrenzung ist belegt." } else { "Technische Rootbegrenzung ist nicht belegt." });
                    let binding = &policy.binding;
                    for (label, text) in [
                        ("Tresor", &binding.vault), ("Speicherort", &binding.location),
                        ("Backend", &binding.backend), ("Konto/Peer", &binding.account),
                        ("Namensraum", &binding.namespace), ("Root", &binding.root),
                    ] { ui.small(format!("{label}: {text}")); }
                    ui.small(format!("Rootobjekt: {}", binding.object.as_deref().unwrap_or("keine Objektkennung verfügbar")));
                    ui.label(if policy.trusted { "Ausdrückliches Vertrauen gilt für diese Speicheridentität." } else { "Für diese Speicheridentität ist kein ausdrückliches Vertrauen gespeichert." });
                    if value.stored.trusted_root.is_some() && !policy.trusted {
                        ui.label("Gespeichertes Vertrauen gilt für eine abweichende Identität und wird nicht verwendet.");
                    }
                    if !policy.enforced && !policy.trusted && ui.add_enabled(!busy, egui::Button::new("Diesem Speicherort für das Einhängen vertrauen…")).clicked() {
                        grant = Some(binding.clone());
                    }
                    ui.small(if policy.read_only { "Der Speicher wird nur lesend eingehängt." } else { "Die Speicherrechte erlauben schreibendes Einhängen." });
                }
                Err(error) => { ui.colored_label(crate::app::theme::danger(ui), format!("Aktuelle Speicheridentität nicht bestätigt: {error}")); }
            }
            if !value.current.as_ref().is_ok_and(|policy| policy.trusted) {
                if let Some(binding) = &value.stored.trusted_root {
                    ui.small(format!("Gespeichertes Vertrauen: Vault {} · Backend {} · Konto/Peer {} · Namensraum {} · Root {} · Speicherort {} · Rootobjekt {}", binding.vault, binding.backend, binding.account, binding.namespace, binding.root, binding.location, binding.object.as_deref().unwrap_or("keine Objektkennung verfügbar")));
                }
            }
            if value.stored.trusted_root.is_some() {
                revoke_root = ui.add_enabled(!busy, egui::Button::new("Speichervertrauen widerrufen…")).clicked();
            }
            ui.separator();
            ui.label(egui::RichText::new("Klartextkopie bei fehlender Laufwerkslaufzeit").strong());
            ui.label(if value.stored.cleartext_copy.is_some() { "Eine separate Klartextkopie-Freigabe ist gespeichert." } else { "Keine dauerhafte Klartextkopie-Freigabe gespeichert." });
            if let Ok(policy) = &value.current {
                if value.stored.cleartext_copy.is_some() && !policy.cleartext_copy {
                    ui.label("Die gespeicherte Klartextfreigabe gilt nicht für die aktuelle Speicheridentität.");
                }
            }
            ui.small(format!("Speicherort: {}", crate::transfer::temp_root().display()));
            ui.small("Offene Bearbeitungen bleiben zur Wiederherstellung nach dem App-Ende erhalten. Automatische oder sichere Löschung ist nicht zugesagt. Sperren verhindert weitere Tresorzugriffe und Rückschreiben; geöffnete Programme können ihre Klartextdaten behalten. Widerruf löscht vorhandene Kopien nicht.");
            ui.small("Eine neue Zustimmung wird ausschließlich beim Öffnen mit tatsächlich fehlender Laufwerkslaufzeit angeboten. Speichervertrauen erteilt keine Klartextfreigabe.");
            if value.stored.cleartext_copy.is_some() {
                revoke_copy = ui.add_enabled(!busy, egui::Button::new("Klartextkopie-Freigabe widerrufen")).clicked();
            }
        }
        if let Some(binding) = grant {
            if crate::app::confirm_yes_no("Diesem Tresorspeicher vertrauen?", &format!("Vault: {}\nBackend: {}\nKonto/Peer: {}\nNamensraum: {}\nRoot: {}\nSpeicherort: {}\n\nDie technische Rootbegrenzung ist nicht belegt. Dieser konkreten Speicheridentität für das Einhängen vertrauen? Diese Wahl erlaubt keine Klartextkopie.", binding.vault, binding.backend, binding.account, binding.namespace, binding.root, binding.location)) {
                self.start(ui.ctx(), site, move |_| crate::vault::confirm_root_trust(&binding));
            }
        }
        if revoke_root && crate::app::confirm_yes_no("Speichervertrauen widerrufen?", "Der Tresor wird gesperrt; offene Laufwerks- und Dokumentanbieterzugriffe werden entzogen. Ungespeicherte Änderungen werden verschlüsselt aufbewahrt. Die getrennte Klartextkopie-Richtlinie bleibt gespeichert.") {
            self.start(ui.ctx(), site, |site| crate::vault::set_root_trust(site, false));
        }
        if revoke_copy { self.start(ui.ctx(), site, |site| crate::vault::set_cleartext_copy_policy(site, false)); }
        if let Some(error) = &self.error { ui.colored_label(crate::app::theme::danger(ui), error); }
        if busy { ui.spinner(); }
        if ui.add_enabled(!busy, egui::Button::new("Speicheridentität erneut abfragen")).clicked() { self.start(ui.ctx(), site, |_| Ok(())); }
    }
}
