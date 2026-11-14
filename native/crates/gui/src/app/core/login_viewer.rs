//! Machine-login pane. A paired device's target (E22) opens without a file
//! and without a pin acknowledgement; the administrative target file stays.
//! Normal device grants never edit the gateway's authority.
use crate::login::{GatewayTarget, ViewerSession};
use crate::share::login_transport::{LoginSwitch, PairedLoginTarget, SwitchAction, ViewerSurface};
use eframe::egui;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "login_viewer_image.rs"]
mod image;
pub struct LoginViewerPane {
    /// The target a paired device sent over its pinned Share session.
    paired: Option<PairedLoginTarget>,
    path: String,
    user: String,
    target: Option<GatewayTarget>,
    acknowledge: bool,
    control: bool,
    session: Option<Arc<ViewerSession>>,
    image: image::LoginImage,
    imported: Arc<Mutex<Option<Result<GatewayTarget, String>>>>,
    loading: bool,
    error: Option<String>,
    shown: Instant,
}
impl Default for LoginViewerPane {
    fn default() -> Self {
        Self {
            paired: None,
            path: String::new(),
            user: String::new(),
            target: None,
            acknowledge: false,
            control: false,
            session: None,
            image: image::LoginImage::default(),
            imported: Arc::new(Mutex::new(None)),
            loading: false,
            error: None,
            shown: Instant::now(),
        }
    }
}
impl LoginViewerPane {
    /// A pane for the target of a paired device.
    pub fn paired(target: PairedLoginTarget) -> Self {
        let mut pane = Self::default();
        pane.set_paired(Some(target));
        pane
    }
    /// Adopts (or drops) a paired device's target; a running session stays.
    pub fn set_paired(&mut self, target: Option<PairedLoginTarget>) {
        if self.paired != target {
            self.paired = target;
        }
    }
    pub fn has_paired(&self) -> bool {
        self.paired.is_some()
    }
    /// Opens the paired device's login screen: no file, no pin
    /// acknowledgement (the pairing pinned the machine key).
    pub fn start_paired(&mut self) -> Result<(), String> {
        let Some(paired) = self.paired.clone() else {
            return Err("login_target_missing: kein Anmeldeziel des Geräts".into());
        };
        self.stop();
        self.error = None;
        self.control = false;
        match ViewerSession::start(paired.target, paired.target_user, true) {
            Ok(session) => {
                self.session = Some(session);
                Ok(())
            }
            Err(error) => {
                let error = error.to_string();
                self.error = Some(error.clone());
                Err(error)
            }
        }
    }
    /// The current session ended (or none runs).
    pub fn ended(&self) -> bool {
        !self
            .session
            .as_ref()
            .is_some_and(|session| session.snapshot().state != "ended")
    }
    /// Why the last session ended, when it did.
    pub fn end_reason(&self) -> Option<String> {
        let snapshot = self.session.as_ref()?.snapshot();
        (snapshot.state == "ended").then(|| snapshot.reason.unwrap_or_default())
    }
    pub fn stop(&mut self) {
        if let Some(session) = self.session.take() {
            session.release();
            session.close();
        }
        self.control = false;
        self.image.reset();
    }
    pub fn cleanup(&mut self) {
        if self.shown.elapsed() > Duration::from_secs(2) {
            self.stop();
        } else if self
            .session
            .as_ref()
            .is_some_and(|session| session.snapshot().state == "ended")
        {
            self.image.reset();
        }
    }
    pub fn needs_tick(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.snapshot().state != "ended")
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.shown = Instant::now();
        if let Some(result) = self.imported.lock().ok().and_then(|mut slot| slot.take()) {
            self.loading = false;
            self.acknowledge = false;
            match result {
                Ok(target) => {
                    self.target = Some(target);
                    self.error = None;
                }
                Err(error) => {
                    self.target = None;
                    self.error = Some(error);
                }
            }
        }
        let snapshot = self.session.as_ref().map(|session| session.snapshot());
        let running = snapshot
            .as_ref()
            .is_some_and(|snapshot| !matches!(snapshot.state.as_str(), "ended" | "stopping"));
        if let Some(paired) = self.paired.clone() {
            ui.label(format!(
                "Anmeldebildschirm von {} · über die gekoppelte Verbindung",
                paired.device_name
            ));
            ui.label(format!(
                "Maschine: {} · Zielkonto: {}",
                paired.target.machine_id, paired.target_user
            ));
            if ui
                .add_enabled(!running, egui::Button::new("Anmeldebildschirm öffnen"))
                .clicked()
            {
                self.image.reset();
                let _ = self.start_paired();
            }
        }
        let administrative = self.paired.is_none() || (running && self.target.is_some());
        egui::CollapsingHeader::new("Administratives Maschinenziel (Datei)")
            .id_salt("login-administrative-target")
            .default_open(administrative)
            .show(ui, |ui| self.administrative(ui, running));
        self.session_view(ui, running);
    }
    fn administrative(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.label("Den öffentlichen Maschinenpin und Zielbenutzer ausdrücklich vom Administrator übernehmen.");
        ui.add_enabled_ui(!running && !self.loading, |ui| {
            ui.horizontal(|ui| {
                ui.label("gateway-target.json:");
                ui.text_edit_singleline(&mut self.path);
                if ui.button("Datei übernehmen").clicked() {
                    let path = self.path.clone();
                    let result = self.imported.clone();
                    self.loading = true;
                    self.acknowledge = false;
                    if let Err(error) = std::thread::Builder::new()
                        .name("login-target-import".into())
                        .spawn(move || {
                            let loaded =
                                crate::login::read_target_file(std::path::Path::new(&path))
                                    .map_err(|error| error.to_string());
                            if let Ok(mut slot) = result.lock() {
                                *slot = Some(loaded);
                            }
                        })
                    {
                        self.loading = false;
                        self.error = Some(error.to_string());
                    }
                }
            });
            if let Some(target) = &self.target {
                ui.label(format!("Maschine: {}", target.machine_id));
                ui.monospace(&target.public_key);
                ui.checkbox(
                    &mut self.acknowledge,
                    "Diesen administrativ übermittelten Maschinenschlüssel ausdrücklich pinnen",
                );
            }
            ui.horizontal(|ui| {
                ui.label("Freigegebene SID/UID:");
                ui.text_edit_singleline(&mut self.user);
            });
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !running && self.target.is_some() && self.acknowledge && !self.user.is_empty(),
                    egui::Button::new("Anmeldung starten"),
                )
                .clicked()
            {
                self.image.reset();
                self.error = None;
                self.control = false;
                if let Some(target) = self.target.clone() {
                    match ViewerSession::start(target, self.user.clone(), self.acknowledge) {
                        Ok(session) => self.session = Some(session),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
        });
    }
    fn session_view(&mut self, ui: &mut egui::Ui, running: bool) {
        if ui
            .add_enabled(running, egui::Button::new("Beenden"))
            .clicked()
        {
            self.stop();
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        let Some(session) = &self.session else {
            return;
        };
        let snapshot = session.snapshot();
        ui.label(format!(
            "Status: {}{}",
            snapshot.state,
            snapshot
                .reason
                .as_ref()
                .map_or(String::new(), |reason| format!(" · {reason}"))
        ));
        if let Some(notice) = &snapshot.notice {
            ui.label(notice);
        }
        if let Some(view) = &snapshot.view {
            ui.label(format!(
                "Maschinenidentität: {} · Zielbenutzer: {}",
                view.target.machine_id, view.target_user
            ));
            if let Some(challenge) = &view.challenge {
                ui.label(format!(
                    "Sitzung {} · {:?} · Policy {}",
                    challenge.binding.session_id,
                    challenge.binding.desktop,
                    challenge.policy_revision
                ));
                ui.collapsing("Boot- und Desktopbindung", |ui| {
                    ui.monospace(&challenge.boot_generation);
                    ui.monospace(&challenge.binding.session_generation);
                    ui.monospace(&challenge.binding.desktop_generation);
                });
            }
            if let Some(access) = &view.access {
                ui.label(format!("Administrative Rechte: Ansicht={} · Steuerung={} · Anmeldung={} · vor Anmeldung={} · Sperre={} · UAC={} · SAS={}",
                    access.rights.screen_view, access.rights.screen_control, access.rights.login, access.rights.before_login, access.rights.locked, access.rights.uac, access.rights.sas));
                ui.add_enabled_ui(running && access.rights.screen_control, |ui| {
                    ui.checkbox(&mut self.control, "Steuern");
                });
                if ui
                    .add_enabled(
                        running
                            && access.rights.sas
                            && access.rights.screen_control
                            && snapshot.frame.is_some(),
                        egui::Button::new("Strg+Alt+Entf anfordern"),
                    )
                    .clicked()
                {
                    if let Some(token) = snapshot.presented_frame.clone() {
                        if let Err(error) =
                            session.secure_attention(token, snapshot.presentation_epoch)
                        {
                            self.error = Some(error);
                        }
                    }
                }
            }
        }
        self.image.show(ui, session, self.control && running);
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }
}
impl Drop for LoginViewerPane {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            session.close();
        }
    }
}
/// The automatic login view of the screen viewer (E22): while the stream of
/// a paired device pauses with `secure_desktop`, `host_locked` or
/// `session_inactive`, the viewer shows the device's login screen without a
/// click, and the picture again as soon as the stream shows the user session.
#[derive(Default)]
pub struct AutoLoginView {
    switch: LoginSwitch,
    pane: Option<LoginViewerPane>,
    cached: Option<(Instant, String, Option<PairedLoginTarget>)>,
    notice: Option<String>,
}
impl AutoLoginView {
    /// The paired target of `selector`, read at most every two seconds.
    fn target(&mut self, selector: &str) -> Option<PairedLoginTarget> {
        if let Some((at, cached, target)) = &self.cached {
            if cached == selector && at.elapsed() < Duration::from_secs(2) {
                return target.clone();
            }
        }
        let target = crate::share::login_transport::paired_login_target(selector).ok();
        self.cached = Some((Instant::now(), selector.to_string(), target.clone()));
        target
    }
    /// One observation of the screen stream of `selector` (device or
    /// contact ID). `EnterLogin`: the caller releases the screen view's held
    /// inputs; `LeaveLogin`: the login view released its inputs and closed.
    pub fn observe(&mut self, selector: &str, screen_state: &str) -> SwitchAction {
        let target = self.target(selector);
        let action = self.switch.observe(screen_state, target.is_some());
        match (action, target) {
            (SwitchAction::EnterLogin, Some(target)) => {
                let mut pane = LoginViewerPane::paired(target);
                match pane.start_paired() {
                    Ok(()) => {
                        self.notice = None;
                        self.pane = Some(pane);
                        return SwitchAction::EnterLogin;
                    }
                    Err(error) => {
                        self.notice = Some(format!("Anmeldebildschirm nicht geöffnet: {error}"));
                        self.switch.login_failed();
                        return SwitchAction::Stay;
                    }
                }
            }
            (SwitchAction::EnterLogin, None) => {
                self.switch.login_failed();
                return SwitchAction::Stay;
            }
            (SwitchAction::LeaveLogin, _) => {
                self.close();
                return SwitchAction::LeaveLogin;
            }
            _ => {}
        }
        if self.switch.surface() == ViewerSurface::Login
            && self.pane.as_ref().is_some_and(LoginViewerPane::ended)
        {
            let reason = self.pane.as_ref().and_then(LoginViewerPane::end_reason);
            self.notice = Some(format!(
                "Anmeldebildschirm beendet{}",
                reason
                    .filter(|reason| !reason.is_empty())
                    .map_or(String::new(), |reason| format!(": {reason}"))
            ));
            self.close();
            self.switch.login_failed();
            return SwitchAction::LeaveLogin;
        }
        SwitchAction::Stay
    }
    /// The login view replaces the picture.
    pub fn active(&self) -> bool {
        self.switch.surface() == ViewerSurface::Login && self.pane.is_some()
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(pane) = &mut self.pane {
            pane.show(ui);
        }
    }
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    /// Another device or the group view: the login view ends.
    pub fn reset(&mut self) {
        self.close();
        self.switch = LoginSwitch::default();
        self.notice = None;
    }
    fn close(&mut self) {
        if let Some(mut pane) = self.pane.take() {
            pane.stop();
        }
    }
}
