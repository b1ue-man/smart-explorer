//! Device-scoped TCP (design 4, user rules r4 „tcp, alles“ and r5 „wenn auf
//! localhost was offen ist wird das frei gegeben für andere geräte. per
//! default“). Every locally listening service of a device that lets this
//! one steer its screen is reachable here automatically, at the device's
//! own loopback address with the same port; the page lists them with a
//! handling each. A further forwarding (a target in the device's network or
//! a port it does not share) keeps its own host grant; it needs no
//! acknowledgement, has no fixed lifetime and waits for the host without a
//! time limit (user rule r7).
use crate::desktop::core::tiles::TileService;
use crate::share::access_constraints::{TcpDirection, TcpHost};
use crate::share::peer_select::present_relation;
use crate::share::tcp_forward::{
    ListenerAdmission, TcpDestination, TcpStart, TunnelId, TunnelRole, TunnelView,
};
use crate::share::{PeerOpenTarget, ShareProfiles};
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The further forwarding beside the automatic services (design 4.5).
const MANUAL_TITLE: &str =
    "Weitere Weiterleitung (Ziel im Netz des Geräts oder nicht freigegebener Port)";
/// Shown while the device does not let this one steer its screen.
pub(crate) const SERVICES_NEED_STEERING: &str = "Die Dienste dieses Geräts erscheinen hier automatisch, sobald es dir „Bildschirm steuern“ erlaubt.";

pub(super) fn device(ui: &mut egui::Ui, profiles: &ShareProfiles, device: &str, key: &str) {
    services(ui, device, key);
    let target = present_relation(profiles, device, key);
    let nodes = super::share_rdp_ui::relation_nodes(profiles, device, key);
    panel(
        ui,
        egui::Id::new(("tcp-device", device, key)),
        &nodes,
        target,
    );
}

/// The services of the device's feed with their address here (design 4.3).
fn services(ui: &mut egui::Ui, device: &str, key: &str) {
    ui.label(egui::RichText::new("Dienste dieses Geräts").strong());
    let feed = match super::share_rdp_ui::feed::device(ui.ctx(), device, key) {
        super::share_rdp_ui::feed::FeedRead::Ready(feed) => feed,
        super::share_rdp_ui::feed::FeedRead::Pending => {
            ui.spinner();
            return;
        }
        super::share_rdp_ui::feed::FeedRead::Unavailable(reason) => {
            ui.label(reason);
            return;
        }
    };
    let Some(feed) = feed else {
        ui.label(
            "Das Gerät ist nicht anwesend; seine Dienste erscheinen, sobald es erreichbar ist.",
        );
        return;
    };
    for note in &feed.notes {
        ui.label(note);
    }
    if !feed.control {
        ui.label(SERVICES_NEED_STEERING);
    }
    for service in &feed.services {
        service_row(ui, service);
    }
}

fn service_row(ui: &mut egui::Ui, service: &TileService) {
    ui.horizontal_wrapped(|ui| {
        ui.label(service.line());
        match (&service.url, service.target()) {
            (Some(url), _) => {
                if ui.button("Öffnen").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
            (None, Some(address)) => {
                if ui.button("Adresse kopieren").clicked() {
                    ui.ctx().copy_text(address);
                }
            }
            (None, None) => {}
        }
    });
}

struct State {
    host: String,
    port: String,
    local_port: String,
    tunnels: Vec<TunnelView>,
    error: Option<String>,
    info: Option<String>,
    busy: bool,
    reading: bool,
    read_at: Option<Instant>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            host: "loopback".into(),
            port: String::new(),
            local_port: "0".into(),
            tunnels: Vec::new(),
            error: None,
            info: None,
            busy: false,
            reading: false,
            read_at: None,
        }
    }
}
type Shared = Arc<Mutex<State>>;

fn panel(ui: &mut egui::Ui, id: egui::Id, nodes: &[&str], target: Option<PeerOpenTarget>) {
    let shared: Shared = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    refresh(ui.ctx(), &shared);
    let mut start = None;
    let mut close = None;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        egui::CollapsingHeader::new(MANUAL_TITLE)
            .id_salt(id.with("manual"))
            .show(ui, |ui| {
                ui.add_enabled_ui(!state.busy, |ui| {
                    egui::Grid::new(id.with("form")).num_columns(2).show(ui, |ui| {
                        let State { host, port, local_port, .. } = &mut *state;
                        for (label, value) in [
                            ("Zielhost (loopback, IP oder DNS)", host),
                            ("Zielport", port),
                            ("Lokaler Port (0 = automatisch)", local_port),
                        ] {
                            ui.label(label);
                            ui.text_edit_singleline(value);
                            ui.end_row();
                        }
                    });
                    ui.label("Das Gerät muss diese Weiterleitung erlauben (Recht „TCP-Weiterleitung“); fragt es nach, wartet der Start auf die Antwort. Die Weiterleitung hat keine feste Frist; nur Programme deines Benutzerkontos erreichen den lokalen Port.");
                    let request = request(&state);
                    if let Err(reason) = &request {
                        ui.label(reason);
                    }
                    if target.is_none() {
                        ui.label(super::share_rdp_ui::OFFLINE);
                    }
                    let enabled = request.is_ok() && target.is_some();
                    if ui
                        .add_enabled(enabled, egui::Button::new("Weiterleitung starten"))
                        .clicked()
                    {
                        start = request.ok().zip(target.clone());
                    }
                });
            });
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Weiterleitung wird gestartet; das Gerät kann nachfragen …");
            });
        }
        if let Some(info) = &state.info {
            ui.label(info);
        }
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        let tunnels: Vec<_> = state
            .tunnels
            .iter()
            .filter(|tunnel| nodes.contains(&tunnel.remote_node.as_str()))
            .collect();
        if !tunnels.is_empty() {
            ui.label(egui::RichText::new("Laufende Weiterleitungen").strong());
        }
        for tunnel in tunnels {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                tunnel_lines(ui, tunnel);
                if ui
                    .add_enabled(
                        !state.busy && !tunnel.closing,
                        egui::Button::new("Schließen"),
                    )
                    .clicked()
                {
                    close = Some(tunnel.id.clone());
                }
            });
        }
    }
    if let Some((request, target)) = start {
        run(ui.ctx(), &shared, move || {
            let tunnel = crate::daemon::tcp_start_within(target, request, None, || {})?;
            Ok(format!(
                "Weiterleitung läuft auf {}",
                tunnel.listener.map(|a| a.to_string()).unwrap_or_default()
            ))
        });
    }
    if let Some(id) = close {
        run(ui.ctx(), &shared, move || close_tunnel(id));
    }
}

fn tunnel_lines(ui: &mut egui::Ui, tunnel: &TunnelView) {
    let role = match tunnel.role {
        TunnelRole::Listener => "Lokaler Zugang",
        TunnelRole::Target => "Zieldienst auf diesem Rechner",
    };
    let direction = match tunnel.direction {
        TcpDirection::PeerToHost => "dieser Rechner → Gerät",
        TcpDirection::HostToPeer => "Gerät → dieser Rechner",
    };
    ui.label(format!(
        "{role} · {direction} · Ziel {} · lokal {}",
        destination_text(&tunnel.destination),
        tunnel
            .listener
            .map(|address| address.to_string())
            .unwrap_or_else(|| "–".into())
    ));
    ui.label(format!(
        "{} · {} · {} Verbindungen · {} B gelesen / {} B geschrieben{}",
        admission_text(tunnel.admission),
        super::share_rdp_ui::lifetime_text(tunnel.expires_at),
        tunnel.active_connections,
        tunnel.socket_read_bytes,
        tunnel.socket_written_bytes,
        if tunnel.closing {
            " · wird geschlossen"
        } else {
            ""
        }
    ));
    if let Some(error) = &tunnel.last_error {
        ui.colored_label(egui::Color32::RED, error);
    }
}

/// Who may use a local listener (design 4.4).
pub(crate) fn admission_text(admission: Option<ListenerAdmission>) -> &'static str {
    match admission {
        Some(ListenerAdmission::SameAccount) => "nur Programme deines Benutzerkontos",
        Some(ListenerAdmission::AwaitProcess) => "nur das gestartete RDP-Programm",
        Some(ListenerAdmission::AnyLocal) => "jedes Programm dieses Geräts",
        None => "Ziel auf diesem Rechner",
    }
}

/// The target of a tunnel in words.
pub(crate) fn destination_text(value: &TcpDestination) -> String {
    match &value.host {
        TcpHost::Loopback => format!("Loopback des Geräts, Port {}", value.port.get()),
        TcpHost::Ip { address } => {
            std::net::SocketAddr::new(*address, value.port.get()).to_string()
        }
        TcpHost::Dns { name, .. } => format!("{name}:{}", value.port.get()),
    }
}

/// The further forwarding: no acknowledgement field (the listener admits
/// only this account's programs) and no lifetime field (0 = no fixed end).
pub(crate) fn manual_request(host: &str, port: &str, local_port: &str) -> Result<TcpStart, String> {
    let value = TcpStart {
        destination: TcpDestination::from_host_port(
            host.trim(),
            port.trim()
                .parse()
                .map_err(|_| "Zielport fehlt oder ist ungültig")?,
        )?,
        local_port: local_port
            .trim()
            .parse()
            .map_err(|_| "Lokaler Port ist ungültig")?,
        acknowledge_local_access: true,
        lifetime_secs: 0,
    };
    value.validate()?;
    Ok(value)
}

fn request(state: &State) -> Result<TcpStart, String> {
    manual_request(&state.host, &state.port, &state.local_port)
}

fn close_tunnel(id: TunnelId) -> Result<String, String> {
    if crate::daemon::tcp_close(id)? {
        Ok("Weiterleitung wird geschlossen.".into())
    } else {
        Err("Diese Weiterleitung ist bereits beendet.".into())
    }
}
fn run(
    ctx: &egui::Context,
    shared: &Shared,
    action: impl FnOnce() -> Result<String, String> + Send + 'static,
) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.error = None;
    }
    let (state, ctx) = (shared.clone(), ctx.clone());
    let retry = state.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("device-tcp-action".into())
        .spawn(move || {
            let result = action();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy = false;
            state.read_at = None;
            match result {
                Ok(info) => {
                    state.info = Some(info);
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
            ctx.request_repaint();
        })
    {
        let mut state = retry.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
        state.error = Some(error.to_string());
    }
}
fn refresh(ctx: &egui::Context, shared: &Shared) {
    ctx.request_repaint_after(Duration::from_secs(1));
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        if state.reading
            || state
                .read_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        state.reading = true;
    }
    let (state, ctx) = (shared.clone(), ctx.clone());
    let retry = state.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("device-tcp-list".into())
        .spawn(move || {
            let result = crate::daemon::tcp_list();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.reading = false;
            state.read_at = Some(Instant::now());
            match result {
                Ok(tunnels) => state.tunnels = tunnels,
                Err(error) => state.error = Some(error),
            }
            ctx.request_repaint();
        })
    {
        let mut state = retry.lock().unwrap_or_else(PoisonError::into_inner);
        state.reading = false;
        state.error = Some(error.to_string());
    }
}
