//! Device-scoped native RDP/RemoteApp controls. Host facts are obtained only
//! through the authenticated daemon producer; UI drafts never become evidence.
use crate::rdp::core::ClientCapability;
use crate::{
    rdp::{AuthenticatedProbe, StartRequest},
    share::{PeerOpenTarget, ShareProfiles},
};
use eframe::egui;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
#[path = "share_rdp_form.rs"]
mod form;
#[path = "share_rdp_sessions.rs"]
mod sessions;

struct Choice {
    target: PeerOpenTarget,
    label: String,
    node: String,
}
struct State {
    probe: Option<Arc<AuthenticatedProbe>>,
    clients: Vec<ClientCapability>,
    service: String,
    client: String,
    app: Option<String>,
    lifetime: String,
    local_ack: bool,
    session_ack: bool,
    certificate_ack: bool,
    busy: bool,
    pending: Option<String>,
    error: Option<String>,
    info: Option<String>,
    loaded: Option<Instant>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            probe: None,
            clients: Vec::new(),
            service: String::new(),
            client: String::new(),
            app: None,
            lifetime: "900".into(),
            local_ack: false,
            session_ack: false,
            certificate_ack: false,
            busy: false,
            pending: None,
            error: None,
            info: None,
            loaded: None,
        }
    }
}
impl State {
    fn reset_consent(&mut self) {
        self.local_ack = false;
        self.session_ack = false;
        self.certificate_ack = false;
    }
}
type Shared = Arc<Mutex<State>>;
type OwnedNodes = Arc<Mutex<BTreeSet<String>>>;

pub(super) fn device(ui: &mut egui::Ui, profiles: &ShareProfiles, device: &str, key: &str) {
    ui.separator();
    ui.heading("Remote Desktop / RemoteApp");
    ui.label("Eigener Zugang über den vorhandenen RDP-Dienst dieses Geräts. Der gemeinsame Bildschirm bleibt separat verfügbar.");
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
    let identity = egui::Id::new(("rdp-relation", device, key));
    let owned_nodes: OwnedNodes = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(identity.with("owned-nodes"), || {
            Arc::new(Mutex::new(BTreeSet::new()))
        })
        .clone()
    });
    let mut selected = ui
        .ctx()
        .data_mut(|d| *d.get_temp_mut_or_insert_with(identity, || 0usize));
    selected = selected.min(choices.len().saturating_sub(1));
    if choices.is_empty() {
        ui.label("Keine gespeicherte ausgehende Direkt- oder Raumverbindung zu dieser bestätigten Geräteidentität.");
    } else {
        egui::ComboBox::from_id_salt(identity)
            .selected_text(&choices[selected].label)
            .show_ui(ui, |ui| {
                for (index, choice) in choices.iter().enumerate() {
                    ui.selectable_value(&mut selected, index, &choice.label);
                }
            });
        ui.ctx().data_mut(|d| d.insert_temp(identity, selected));
        panel(ui, identity, &choices[selected], &owned_nodes);
    }
    let active_nodes: BTreeSet<_> = crate::rdp::owned_sessions()
        .into_iter()
        .map(|s| s.tunnel.remote_node)
        .collect();
    let observed = {
        let mut nodes = owned_nodes.lock().unwrap_or_else(PoisonError::into_inner);
        nodes.retain(|node| active_nodes.contains(node));
        nodes.iter().cloned().collect::<Vec<_>>()
    };
    let nodes: Vec<_> = choices
        .iter()
        .map(|c| c.node.as_str())
        .chain(
            profiles
                .direct_grants
                .iter()
                .filter(|grant| grant.device_id == device && grant.public_key == key)
                .map(|grant| grant.node_id.as_str()),
        )
        .chain(observed.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .collect();
    sessions::show(ui, &nodes);
}
fn panel(ui: &mut egui::Ui, identity: egui::Id, choice: &Choice, owned_nodes: &OwnedNodes) {
    let id = identity.with((choice.target.endpoint_prefix(), &choice.node));
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    let refresh;
    let mut start = None;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        refresh = ui
            .add_enabled(
                !state.busy,
                egui::Button::new("RDP-Fähigkeiten abfragen / aktualisieren"),
            )
            .clicked();
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(state.pending.as_deref().unwrap_or("RDP-Anfrage läuft …"));
            });
        }
        if state.probe.is_none() && !state.busy {
            ui.label("Die RDP-Hostfähigkeit und der lokale Client wurden noch nicht abgefragt.");
        }
        if let Some(at) = state.loaded {
            ui.label(format!(
                "Authentifizierte Probe vor {} s; spätestens nach fünf Minuten erneut abfragen.",
                at.elapsed().as_secs()
            ));
        }
        if let Some(info) = &state.info {
            ui.label(info);
        }
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(probe) = state.probe.clone() {
            ui.add_enabled_ui(!state.busy, |ui| {
                start = form::show(ui, id, &probe, &mut state);
            });
        }
    }
    if refresh {
        load(ui.ctx(), &shared, choice.target.clone());
    }
    if let Some((probe, request)) = start {
        launch(
            ui.ctx(),
            &shared,
            choice.target.clone(),
            probe,
            request,
            owned_nodes.clone(),
        );
    }
}
fn load(ctx: &egui::Context, shared: &Shared, target: PeerOpenTarget) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.pending = Some("Host und vorhandene lokale Clients werden abgefragt …".into());
        state.error = None;
        state.probe = None;
        state.reset_consent();
    }
    let (state, context) = (shared.clone(), ctx.clone());
    let retry = state.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("device-rdp-probe".into())
        .spawn(move || {
            let clients = crate::rdp::local_clients();
            let report =
                crate::daemon::apps_rdp_probe(target, Some(Duration::from_secs(30)), || {
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.pending = Some("Warte auf Bestätigung am Zielgerät …".into());
                    context.request_repaint();
                });
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy = false;
            state.pending = None;
            state.clients = clients;
            match report {
                Ok(report) => {
                    state.service = report
                        .report()
                        .services
                        .first()
                        .map(|s| s.id.clone())
                        .unwrap_or_default();
                    state.client = state
                        .clients
                        .iter()
                        .find(|c| c.available)
                        .or_else(|| state.clients.first())
                        .map(|c| c.id.clone())
                        .unwrap_or_default();
                    state.app = None;
                    state.probe = Some(Arc::new(report));
                    state.loaded = Some(Instant::now());
                }
                Err(error) => {
                    state.error = Some(error);
                    state.loaded = None;
                }
            }
            context.request_repaint();
        })
    {
        let mut state = retry.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
        state.pending = None;
        state.error = Some(error.to_string());
    }
}
fn launch(
    ctx: &egui::Context,
    shared: &Shared,
    target: PeerOpenTarget,
    probe: Arc<AuthenticatedProbe>,
    request: StartRequest,
    owned_nodes: OwnedNodes,
) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.pending = Some("RDP-Zugang wird bestätigt und der native Client gestartet …".into());
        state.error = None;
        state.reset_consent();
    }
    let (state, context) = (shared.clone(), ctx.clone());
    let retry = state.clone();
    if let Err(error)=std::thread::Builder::new().name("device-rdp-start".into()).spawn(move || {
        let result=crate::rdp::start_owned(target,&probe,&request,Duration::from_secs(30),|| {
            let mut state=state.lock().unwrap_or_else(PoisonError::into_inner);state.pending=Some("Warte auf die TCP-Freigabe am Zielgerät …".into());context.request_repaint();
        });
        // The node comes from the admitted, authenticated tunnel receipt,
        // never from a UI draft. Retain it across relation/presence changes.
        if let Ok(session)=&result {owned_nodes.lock().unwrap_or_else(PoisonError::into_inner).insert(session.tunnel.remote_node.clone());}
        let mut state=state.lock().unwrap_or_else(PoisonError::into_inner);state.busy=false;state.pending=None;
        match result {Ok(session)=>state.info=Some(format!("Nativer RDP-Client gestartet; Sitzung {}. Zugangsdaten ausschließlich im Client eingeben.",session.tunnel.id.as_str())),Err(error)=>state.error=Some(error)}
        context.request_repaint();
    }) {let mut state=retry.lock().unwrap_or_else(PoisonError::into_inner);state.busy=false;state.pending=None;state.error=Some(error.to_string());}
}
