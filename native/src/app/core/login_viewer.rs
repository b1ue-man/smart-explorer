//! Dedicated machine-login pane; normal device grants never edit its authority.
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};
use eframe::egui;
use crate::login::{GatewayTarget, ViewerSession};
#[path = "login_viewer_image.rs"] mod image;
pub(crate) struct LoginViewerPane {
    path: String, user: String, target: Option<GatewayTarget>, acknowledge: bool, control: bool,
    session: Option<Arc<ViewerSession>>, image: image::LoginImage,
    imported: Arc<Mutex<Option<Result<GatewayTarget, String>>>>, loading: bool, error: Option<String>,
    shown: Instant,
}
impl Default for LoginViewerPane {
    fn default() -> Self { Self { path: String::new(), user: String::new(), target: None, acknowledge: false,
        control: false, session: None, image: image::LoginImage::default(), imported: Arc::new(Mutex::new(None)), loading: false, error: None, shown: Instant::now() } }
}
impl LoginViewerPane {
    pub(crate) fn stop(&mut self) { if let Some(session) = self.session.take() { session.release(); session.close(); } self.control = false; self.image.reset(); }
    pub(crate) fn cleanup(&mut self) {
        if self.shown.elapsed() > Duration::from_secs(2) { self.stop(); }
        else if self.session.as_ref().is_some_and(|session| session.snapshot().state == "ended") { self.image.reset(); }
    }
    pub(crate) fn needs_tick(&self) -> bool { self.session.as_ref().is_some_and(|session| session.snapshot().state != "ended") }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        self.shown = Instant::now();
        if let Some(result) = self.imported.lock().ok().and_then(|mut slot| slot.take()) {
            self.loading = false; self.acknowledge = false;
            match result { Ok(target) => { self.target = Some(target); self.error = None; }, Err(error) => { self.target = None; self.error = Some(error); } }
        }
        let snapshot = self.session.as_ref().map(|session| session.snapshot());
        let running = snapshot.as_ref().is_some_and(|snapshot| !matches!(snapshot.state.as_str(), "ended" | "stopping"));
        ui.label("Maschinenanmeldung · eigener administrativer Zugang");
        ui.label("Die normale Gerätefreigabe erteilt hier keine Rechte. Den öffentlichen Maschinenpin und Zielbenutzer ausdrücklich vom Administrator übernehmen.");
        ui.add_enabled_ui(!running && !self.loading, |ui| {
            ui.horizontal(|ui| {
                ui.label("gateway-target.json:"); ui.text_edit_singleline(&mut self.path);
                if ui.button("Datei übernehmen").clicked() {
                    let path = self.path.clone(); let result = self.imported.clone(); self.loading = true; self.acknowledge = false;
                    if let Err(error) = std::thread::Builder::new().name("login-target-import".into()).spawn(move || {
                        let loaded = crate::login::read_target_file(std::path::Path::new(&path)).map_err(|error| error.to_string());
                        if let Ok(mut slot) = result.lock() { *slot = Some(loaded); }
                    }) { self.loading = false; self.error = Some(error.to_string()); }
                }
            });
            if let Some(target) = &self.target {
                ui.label(format!("Maschine: {}", target.machine_id)); ui.monospace(&target.public_key);
                ui.checkbox(&mut self.acknowledge, "Diesen administrativ übermittelten Maschinenschlüssel ausdrücklich pinnen");
            }
            ui.horizontal(|ui| { ui.label("Freigegebene SID/UID:"); ui.text_edit_singleline(&mut self.user); });
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(!running && self.target.is_some() && self.acknowledge && !self.user.is_empty(), egui::Button::new("Anmeldung starten")).clicked() {
                self.image.reset(); self.error = None; self.control = false;
                if let Some(target) = self.target.clone() { match ViewerSession::start(target, self.user.clone(), self.acknowledge) {
                    Ok(session) => self.session = Some(session), Err(error) => self.error = Some(error.to_string()),
                } }
            }
            if ui.add_enabled(running, egui::Button::new("Beenden")).clicked() { self.stop(); }
        });
        if let Some(error) = &self.error { ui.colored_label(egui::Color32::RED, error); }
        let Some(session) = &self.session else { return; };
        let snapshot = session.snapshot();
        ui.label(format!("Status: {}{}", snapshot.state, snapshot.reason.as_ref().map_or(String::new(), |reason| format!(" · {reason}"))));
        if let Some(notice) = &snapshot.notice { ui.label(notice); }
        if let Some(view) = &snapshot.view {
            ui.label(format!("Maschinenidentität: {} · Zielbenutzer: {}", view.target.machine_id, view.target_user));
            if let Some(challenge) = &view.challenge { ui.label(format!("Sitzung {} · {:?} · Policy {}", challenge.binding.session_id, challenge.binding.desktop, challenge.policy_revision));
                ui.collapsing("Boot- und Desktopbindung", |ui| { ui.monospace(&challenge.boot_generation); ui.monospace(&challenge.binding.session_generation); ui.monospace(&challenge.binding.desktop_generation); }); }
            if let Some(access) = &view.access {
                ui.label(format!("Administrative Rechte: Ansicht={} · Steuerung={} · Anmeldung={} · vor Anmeldung={} · Sperre={} · UAC={} · SAS={}",
                    access.rights.screen_view, access.rights.screen_control, access.rights.login, access.rights.before_login, access.rights.locked, access.rights.uac, access.rights.sas));
                ui.add_enabled_ui(running && access.rights.screen_control, |ui| { ui.checkbox(&mut self.control, "Steuern"); });
                if ui.add_enabled(running && access.rights.sas && access.rights.screen_control && snapshot.frame.is_some(), egui::Button::new("Strg+Alt+Entf anfordern")).clicked() {
                    if let Some(token) = snapshot.presented_frame.clone() { if let Err(error) = session.secure_attention(token, snapshot.presentation_epoch) { self.error = Some(error); } }
                }
            }
        }
        self.image.show(ui, session, self.control && running);
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
}
impl Drop for LoginViewerPane { fn drop(&mut self) { if let Some(session) = &self.session { session.close(); } } }
