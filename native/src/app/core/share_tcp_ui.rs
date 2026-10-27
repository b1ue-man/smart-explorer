//! Device-scoped TCP controls. Drafts are local UI state; only the remote
//! host's AccessV2 admission grants access to its destination service.
use crate::share::access_constraints::{TcpDirection, TcpHost};
use crate::share::tcp_forward::{
    TcpDestination, TcpStart, TunnelId, TunnelView, TCP_LOCAL_ACCESS_NOTICE,
};
use crate::share::{PeerOpenTarget, ShareProfiles};
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

struct Choice {
    target: PeerOpenTarget,
    label: String,
    node: String,
}
pub(super) fn device(ui: &mut egui::Ui, profiles: &ShareProfiles, device: &str, key: &str) {
    let mut choices = Vec::new();
    for contact in &profiles.direct_contacts {
        if contact.remote_device_id.as_deref() == Some(device)
            && contact.remote_public_key.as_deref() == Some(key)
        {
            choices.push(Choice {
                target: PeerOpenTarget::Direct {
                    contact_id: contact.id.clone(),
                },
                label: format!("Direkt: {}", contact.display_name),
                node: if contact.expected_node_id.is_empty() {
                    contact
                        .presence
                        .as_ref()
                        .map(|p| p.node_id.clone())
                        .unwrap_or_default()
                } else {
                    contact.expected_node_id.clone()
                },
            });
        }
    }
    for room in &profiles.rooms {
        for member in &room.members {
            if member.device_id == device && member.public_key == key && !member.blocked {
                choices.push(Choice {
                    target: PeerOpenTarget::RoomDevice {
                        room_id: room.room_id.clone(),
                        device_id: device.into(),
                    },
                    label: format!("Raum „{}“ / {}", room.name, member.device_name),
                    node: member.node_id.clone(),
                });
            }
        }
    }
    let selected_id = egui::Id::new(("tcp-device-relation", device, key));
    let mut selected = ui
        .ctx()
        .data_mut(|data| *data.get_temp_mut_or_insert_with(selected_id, || 0usize));
    selected = selected.min(choices.len().saturating_sub(1));
    if !choices.is_empty() {
        let labels: Vec<&str> = choices.iter().map(|choice| choice.label.as_str()).collect();
        super::device_page_layout::relation_picker(ui, &labels, &mut selected);
        ui.ctx()
            .data_mut(|data| data.insert_temp(selected_id, selected));
        let choice = &choices[selected];
        panel(ui, selected_id, &choice.node, Some(choice.target.clone()));
    } else {
        ui.label("Keine gespeicherte ausgehende Direkt- oder Raumverbindung zu diesem Gerät.");
        let node = profiles
            .direct_grants
            .iter()
            .find(|grant| grant.device_id == device && grant.public_key == key)
            .map(|grant| grant.node_id.as_str())
            .unwrap_or("");
        panel(ui, selected_id, node, None);
    }
}

struct State {
    host: String,
    port: String,
    local_port: String,
    lifetime: String,
    acknowledged: bool,
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
            lifetime: "900".into(),
            acknowledged: false,
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
fn panel(ui: &mut egui::Ui, identity: egui::Id, peer: &str, target: Option<PeerOpenTarget>) {
    let id = egui::Id::new((
        "tcp-device",
        identity,
        peer,
        target.as_ref().map(PeerOpenTarget::endpoint_prefix),
    ));
    let shared: Shared = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    refresh(ui.ctx(), &shared);
    let mut start = None;
    let mut close = None;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(target) = target {
            ui.label("Neuer lokaler Zugang zum Dienst dieses Geräts");
            ui.add_enabled_ui(!state.busy, |ui| {
                egui::Grid::new(id.with("form")).num_columns(2).show(ui, |ui| {
                    let mut changed = false;
                    let State { host, port, local_port, lifetime, .. } = &mut *state;
                    for (label, value) in [("Zielhost (loopback, IP oder DNS)", host), ("Zielport", port),
                        ("Lokaler Port (0 = automatisch)", local_port), ("Lebensdauer in Sekunden (1–86400)", lifetime)] {
                        ui.label(label); changed |= ui.text_edit_singleline(value).changed(); ui.end_row();
                    }
                    if changed { state.acknowledged = false; }
                });
                ui.add(egui::Label::new(TCP_LOCAL_ACCESS_NOTICE).wrap());
                ui.checkbox(&mut state.acknowledged, "Ich bestätige diesen lokalen Zugriff.");
                ui.label("Das Zielgerät muss genau diese Beziehung, Richtung, Zieladresse und diesen Port erlauben. Die Gegenrichtung wird dort separat gestartet.");
                ui.label("Auf eine Startbestätigung wird bis zu 30 Sekunden gewartet; danach wird dieser Start abgebrochen.");
                let request = request(&state);
                if ui.add_enabled(request.is_ok() && !peer.is_empty(), egui::Button::new("TCP-Weiterleitung starten")).clicked() {
                    start = request.ok().map(|request| (target, request));
                }
            });
        }
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("TCP-Anfrage läuft …");
            });
        }
        if let Some(info) = &state.info {
            ui.label(info);
        }
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        ui.label("Aktive Weiterleitungen dieses Geräts");
        let tunnels: Vec<_> = state
            .tunnels
            .iter()
            .filter(|tunnel| !peer.is_empty() && tunnel.remote_node == peer)
            .collect();
        if tunnels.is_empty() {
            ui.label("Keine aktive Weiterleitung.");
        }
        for tunnel in tunnels {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let role = match tunnel.role { crate::share::tcp_forward::TunnelRole::Listener => "Lokaler Zugang", crate::share::tcp_forward::TunnelRole::Target => "Zieldienst auf diesem Gerät" };
                ui.label(format!("ID: {} · {role} · {} / {}", tunnel.id.as_str(), tunnel.relation_kind, tunnel.relation_id));
                let direction = match tunnel.direction { TcpDirection::PeerToHost => "Anfragendes Gerät → Zielhost", TcpDirection::HostToPeer => "Host → Gegenstelle" };
                ui.label(format!("Initiator: {}\nZielgerät: {}\nRichtung: {direction}\nZieldienst: {}\nGebundene Adresse: {}\nLokaler Listener: {}",
                    tunnel.initiator_node, tunnel.target_node, destination(&tunnel.destination),
                    tunnel.bound_target.map(|address| address.to_string()).unwrap_or_else(|| "–".into()),
                    tunnel.listener.map(|address| address.to_string()).unwrap_or_else(|| "–".into())));
                let left = tunnel.expires_at.saturating_sub(crate::share::core_now_secs()).max(0);
                ui.label(format!("Spätester Ablauf: in {left} s · {} aktive Verbindungen · {} B gelesen / {} B geschrieben{}",
                    tunnel.active_connections, tunnel.socket_read_bytes, tunnel.socket_written_bytes,
                    if tunnel.closing { " · wird geschlossen" } else { "" }));
                if let Some(error) = &tunnel.last_error { ui.colored_label(egui::Color32::RED, error); }
                if ui.add_enabled(!state.busy && !tunnel.closing, egui::Button::new("Schließen")).clicked() { close = Some(tunnel.id.clone()); }
            });
        }
    }
    if let Some((target, request)) = start {
        run(ui.ctx(), &shared, move || {
            let tunnel = crate::daemon::tcp_start_within(
                target,
                request,
                Some(Duration::from_secs(30)),
                || {},
            )?;
            Ok(format!(
                "Weiterleitung {} läuft auf {}",
                tunnel.id.as_str(),
                tunnel.listener.map(|a| a.to_string()).unwrap_or_default()
            ))
        });
    }
    if let Some(id) = close {
        run(ui.ctx(), &shared, move || close_tunnel(id));
    }
}
fn destination(value: &TcpDestination) -> String {
    match &value.host {
        TcpHost::Loopback => format!("Loopback des Zielgeräts · Port {}", value.port.get()),
        TcpHost::Ip { address } => {
            std::net::SocketAddr::new(*address, value.port.get()).to_string()
        }
        TcpHost::Dns { name, .. } => format!("{name}:{}", value.port.get()),
    }
}
fn request(state: &State) -> Result<TcpStart, String> {
    let value = TcpStart {
        destination: TcpDestination::from_host_port(
            state.host.trim(),
            state
                .port
                .parse()
                .map_err(|_| "Zielport fehlt oder ist ungültig")?,
        )?,
        local_port: state
            .local_port
            .parse()
            .map_err(|_| "Lokaler Port ist ungültig")?,
        lifetime_secs: state
            .lifetime
            .parse()
            .map_err(|_| "Lebensdauer ist ungültig")?,
        acknowledge_local_access: state.acknowledged,
    };
    value.validate()?;
    Ok(value)
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
        state.acknowledged = false;
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
