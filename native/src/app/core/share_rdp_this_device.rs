//! Page „Dieses Gerät“: whether other devices reach this device's Remote
//! Desktop and which of this device's services they use right now (design
//! 3.4 and 8 T-L1). The background service sets Remote Desktop up by itself
//! when a device receives „Bildschirm steuern: Erlauben“; this page reads
//! the result (the same read-only probe the service uses) and lists every
//! running target tunnel with a handling to close it.
use crate::rdp::HostProbe;
use crate::share::tcp_forward::{TunnelId, TunnelRole, TunnelView};
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The probe is read again at most this often while the page shows (a
/// display rate: the Windows probe starts PowerShell).
const PROBE_EVERY: Duration = Duration::from_secs(60);
/// The running target tunnels are read at this display rate.
const TUNNELS_EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct State {
    probe: Option<HostProbe>,
    probe_at: Option<Instant>,
    probing: bool,
    tunnels: Vec<TunnelView>,
    tunnels_at: Option<Instant>,
    listing: bool,
    error: Option<String>,
}
type Shared = Arc<Mutex<State>>;

/// The lines of this device's Remote Desktop state.
pub(crate) fn probe_lines(probe: &HostProbe) -> Vec<String> {
    let mut lines = Vec::new();
    match probe.preferred_service() {
        Some(service) => lines.push(format!(
            "Remote Desktop dieses Geräts ist für Geräte mit „Bildschirm steuern: Erlauben“ bereit ({}, Port {}).",
            service.id,
            service.port.map_or_else(|| "–".into(), |port| port.to_string())
        )),
        None => lines.push(
            "Remote Desktop dieses Geräts ist noch nicht startbar; der Hintergrunddienst richtet es ein, sobald ein Gerät „Bildschirm steuern: Erlauben“ hat und erscheint."
                .into(),
        ),
    }
    lines.extend(probe.unavailable_reason.iter().cloned());
    for service in &probe.services {
        if let Some(reason) = &service.unavailable_reason {
            lines.push(format!("{}: {reason}", service.id));
        }
    }
    lines.extend(probe.diagnostics.iter().cloned());
    lines
}

pub(crate) fn this_device(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Remote Desktop und Dienste für andere Geräte").strong());
    let id = egui::Id::new("this-device-rdp");
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    refresh(ui.ctx(), &shared);
    let mut close = None;
    {
        let state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        match &state.probe {
            Some(probe) => {
                for line in probe_lines(probe) {
                    ui.label(line);
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Remote Desktop dieses Geräts wird geprüft …");
                });
            }
        }
        let targets: Vec<&TunnelView> = state
            .tunnels
            .iter()
            .filter(|tunnel| tunnel.role == TunnelRole::Target)
            .collect();
        if targets.is_empty() {
            ui.label("Gerade nutzt kein anderes Gerät einen Dienst dieses Geräts.");
        }
        for tunnel in targets {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Genutzt von {} · Ziel {} · {} Verbindungen",
                    tunnel.initiator_node,
                    super::super::share_tcp_ui::destination_text(&tunnel.destination),
                    tunnel.active_connections
                ));
                if ui
                    .add_enabled(!tunnel.closing, egui::Button::new("Beenden"))
                    .clicked()
                {
                    close = Some(tunnel.id.clone());
                }
            });
        }
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }
    if let Some(id) = close {
        close_tunnel(ui.ctx(), &shared, id);
    }
}

fn refresh(ctx: &egui::Context, shared: &Shared) {
    ctx.request_repaint_after(TUNNELS_EVERY);
    let (probe, list) = {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let probe = !state.probing && state.probe_at.is_none_or(|at| at.elapsed() >= PROBE_EVERY);
        let list = !state.listing
            && state
                .tunnels_at
                .is_none_or(|at| at.elapsed() >= TUNNELS_EVERY);
        state.probing |= probe;
        state.listing |= list;
        (probe, list)
    };
    if probe {
        spawn(ctx, shared, "this-device-rdp-probe", |state| {
            let probe = crate::rdp::probe_local_host();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.probe = Some(probe);
            state.probe_at = Some(Instant::now());
            state.probing = false;
        });
    }
    if list {
        spawn(ctx, shared, "this-device-tunnels", |state| {
            let result = crate::daemon::tcp_list();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.tunnels_at = Some(Instant::now());
            state.listing = false;
            match result {
                Ok(tunnels) => state.tunnels = tunnels,
                Err(error) => state.error = Some(error),
            }
        });
    }
}

fn close_tunnel(ctx: &egui::Context, shared: &Shared, id: TunnelId) {
    spawn(ctx, shared, "this-device-tunnel-close", move |state| {
        let result = crate::daemon::tcp_close(id);
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.tunnels_at = None;
        if let Err(error) = result {
            state.error = Some(error);
        }
    });
}

fn spawn(
    ctx: &egui::Context,
    shared: &Shared,
    name: &str,
    work: impl FnOnce(&Shared) + Send + 'static,
) {
    let (state, context) = (shared.clone(), ctx.clone());
    if let Err(error) = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            work(&state);
            context.request_repaint();
        })
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.probing = false;
        state.listing = false;
        state.error = Some(error.to_string());
    }
}
