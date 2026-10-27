//! Device-page entry point and a read-only local administrative host indicator.
use super::login_viewer::LoginViewerPane;
use eframe::egui;
use std::{
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
static PANES: Mutex<Vec<Weak<Mutex<LoginViewerPane>>>> = Mutex::new(Vec::new());
pub(crate) fn tick(ctx: &egui::Context) {
    let mut active = false;
    if let Ok(mut panes) = PANES.lock() {
        panes.retain(|weak| {
            if let Some(pane) = weak.upgrade() {
                if let Ok(mut pane) = pane.lock() {
                    pane.cleanup();
                    active |= pane.needs_tick();
                }
                true
            } else {
                false
            }
        });
    }
    if active {
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}
pub(super) fn device(ui: &mut egui::Ui, device: &str, ordinary_key: &str) {
    // This key only scopes the UI draft. It never becomes a machine pin/grant.
    let id = egui::Id::new(("explicit-login-machine", device, ordinary_key));
    let pane: Arc<Mutex<LoginViewerPane>> = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || {
            let pane = Arc::new(Mutex::new(LoginViewerPane::default()));
            if let Ok(mut panes) = PANES.lock() {
                panes.push(Arc::downgrade(&pane));
            }
            pane
        })
        .clone()
    });
    let mut open = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id.with("open")))
        .unwrap_or(false);
    if ui
        .button("Administratives Maschinenziel übernehmen / Anmeldung öffnen")
        .clicked()
    {
        open = true;
    }
    if open {
        egui::Window::new("Maschinenanmeldung")
            .id(id.with("window"))
            .open(&mut open)
            .resizable(true)
            .default_size(egui::vec2(960.0, 760.0))
            .max_size(ui.ctx().screen_rect().size() * 0.95)
            .show(ui.ctx(), |ui| {
                if let Ok(mut pane) = pane.lock() {
                    pane.show(ui);
                }
            });
    }
    if !open {
        if let Ok(mut pane) = pane.lock() {
            pane.stop();
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("open"), open));
}
struct HostState {
    at: Instant,
    checked: Instant,
    recheck: bool,
    busy: bool,
    status: Option<crate::login::InstallationStatus>,
}
pub(crate) fn host_indicator(ui: &mut egui::Ui) {
    let shared: Arc<Mutex<HostState>> = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::new("login-host-indicator"), || {
            Arc::new(Mutex::new(HostState {
                at: Instant::now() - Duration::from_secs(10),
                checked: Instant::now(),
                recheck: true,
                busy: false,
                status: None,
            }))
        })
        .clone()
    });
    let mut state = match shared.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    if !state.busy && state.at.elapsed() > Duration::from_secs(1) {
        state.busy = true;
        let recheck = state.recheck
            || state.status.is_none()
            || state.checked.elapsed() >= Duration::from_secs(30);
        state.recheck = false;
        let cached = state.status.clone();
        let result = shared.clone();
        let context = ui.ctx().clone();
        if std::thread::Builder::new()
            .name("login-host-status".into())
            .spawn(move || {
                let status = if recheck {
                    crate::login::installation_status()
                } else {
                    let mut status = cached.unwrap_or_else(crate::login::installation_status);
                    status.activity = crate::login::host_activity();
                    status.service_runtime = status.activity.state.clone();
                    status
                };
                if let Ok(mut state) = result.lock() {
                    state.status = Some(status);
                    state.at = Instant::now();
                    if recheck {
                        state.checked = Instant::now();
                    }
                    state.busy = false;
                }
                context.request_repaint();
            })
            .is_err()
        {
            state.busy = false;
            state.at = Instant::now();
        }
    }
    ui.label("Lokale Maschinenanmeldung · Rechte ausschließlich administrativ verwaltet");
    if ui
        .add_enabled(
            !state.busy,
            egui::Button::new("Installationsbestand erneut prüfen"),
        )
        .clicked()
    {
        state.recheck = true;
        state.at = Instant::now() - Duration::from_secs(2);
    }
    if let Some(status) = &state.status {
        ui.label(format!(
            "Installation: {} · Dienstnachweis: {}",
            status.state, status.service_runtime
        ));
        ui.label(format!(
            "Installationsbestand vor {} s geprüft; Aktivitätsbeleg wird gesondert aktualisiert.",
            state.checked.elapsed().as_secs()
        ));
        if let Some(reason) = &status.reason {
            ui.label(reason);
        }
        if let Some(policy) = &status.policy {
            ui.label(format!(
                "Policy {} · {} administrativ gespeicherte Gegenstellen",
                policy.revision,
                policy.grants.len()
            ));
        }
        if let Some(evidence) = &status.activity.evidence {
            ui.label(format!("Geschützter Brokerbeleg: {} · Policy {} · {} administrative Gegenstellen · gültig bis {} ms UTC", evidence.state,
                evidence.policy_revision, evidence.administrative_grants, evidence.valid_until_ms));
        }
        if let Some(reason) = &status.activity.reason {
            ui.label(format!("Aktivität nicht aktuell belegt: {reason}"));
        }
        ui.label("Installation und ausgehende Viewer belegen keine lokale Aktivität. Aktiv wird nur mit geschütztem, frischem Broker-/Helferprozessbeleg angezeigt.");
    } else {
        ui.label("Installations- und Dienstzustand noch unbekannt.");
    }
    ui.ctx().request_repaint_after(Duration::from_secs(1));
}
