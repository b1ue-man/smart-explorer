//! Device-scoped Remote Desktop (design 2, user rule r3 „die eine freigabe
//! muss reichen, der rest muss laufen“): one handling „Remote Desktop“ and
//! one per published RemoteApp. se derives the present relation, a fresh
//! authenticated probe, the host's setup, the service, the client, the
//! certificate trust and the session's life (`rdp::open_device`); there is
//! no form, no acknowledgement and no lifetime field. Host facts come only
//! from the authenticated feed and probe; UI state never becomes evidence.
use crate::desktop::core::tiles::{self, RdpEntry};
use crate::rdp::core::ClientCapability;
use crate::share::peer_select::present_relation;
use crate::share::ShareProfiles;
use eframe::egui;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
#[path = "device_feed_cache.rs"]
pub(super) mod feed;
#[path = "share_rdp_login_ui.rs"]
mod login;
#[path = "share_rdp_sessions.rs"]
mod sessions;
pub use sessions::lifetime_text;
#[path = "share_rdp_this_device.rs"]
mod this_device;
pub use this_device::this_device;

/// Shown while a start runs; the host may ask once for administrator
/// approval when it sets Remote Desktop up (design 3.1 T4, T-R6).
pub const OPENING: &str = "Remote Desktop wird geöffnet … Richtet das Gerät Remote Desktop dabei erst ein, fragt es dort einmal nach der Administratorfreigabe (Windows: Benutzerkontensteuerung, Linux: polkit).";
/// No present relation to the device (design 2.1 step 1).
pub const OFFLINE: &str = "Gerät ist nicht erreichbar";

#[derive(Default)]
struct State {
    busy: bool,
    error: Option<String>,
    info: Option<String>,
    clients: Option<Vec<ClientCapability>>,
    clients_reading: bool,
    /// The start that waits for typed account credentials (Linux without
    /// `sdl-freerdp3`, design 2.8): its RemoteApp, `None` = whole desktop.
    ask: Option<Option<String>>,
    user: String,
    domain: String,
    password: String,
    /// P4: the sign-in with the device's account (state line, dialog).
    login: login::Panel,
}
type Shared = Arc<Mutex<State>>;
type OwnedNodes = Arc<Mutex<BTreeSet<String>>>;

/// The buttons of the device page: exactly the handlings of
/// [`tiles::rdp_entries`], nothing else (design 10.3 U1).
pub fn page_entries(feed: Option<&tiles::TileFeed>) -> (Vec<RdpEntry>, Option<String>) {
    tiles::rdp_entries(feed.and_then(|feed| feed.rdp.as_ref()))
}

/// Why no local client can start, or `None` when one can.
pub fn client_gap(clients: &[ClientCapability]) -> Option<String> {
    if clients.iter().any(|client| client.available) {
        return None;
    }
    let details: Vec<&str> = clients
        .iter()
        .map(|client| client.detail.as_str())
        .collect();
    Some(if details.is_empty() {
        "Auf diesem Rechner ist kein RDP-Programm vorhanden (Windows: mstsc, Linux: sdl-freerdp3 aus dem Paket freerdp3-sdl).".into()
    } else {
        format!("Kein startbares RDP-Programm: {}", details.join(" · "))
    })
}

/// One button per handling, nothing else: no field, no checkbox (design
/// 10.3 U1). Returns the clicked entry's RemoteApp (`Some(None)`: the whole
/// desktop).
pub fn entry_buttons(
    ui: &mut egui::Ui,
    entries: &[RdpEntry],
    enabled: bool,
) -> Option<Option<String>> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for entry in entries {
            if ui
                .add_enabled(enabled, egui::Button::new(entry.label.as_str()))
                .clicked()
            {
                chosen = Some(entry.remote_app.clone());
            }
        }
    });
    chosen
}

pub(super) fn device(ui: &mut egui::Ui, profiles: &ShareProfiles, device: &str, key: &str) {
    ui.label("Öffnet den Desktop dieses Geräts im RDP-Programm dieses Rechners. se richtet beide Seiten selbst ein und meldet mit dem Konto an, das die Freigabe des Geräts nennt; braucht das ein Kennwort, fragt diese Seite einmal danach, und das Gerät speichert es.");
    let identity = egui::Id::new(("rdp-device", device, key));
    let shared: Shared = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(identity, || Arc::new(Mutex::new(State::default())))
            .clone()
    });
    let owned_nodes: OwnedNodes = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_insert_with(identity.with("owned-nodes"), || {
            Arc::new(Mutex::new(BTreeSet::new()))
        })
        .clone()
    });
    read_clients(ui.ctx(), &shared);
    let feed = match feed::device(ui.ctx(), device, key) {
        feed::FeedRead::Ready(feed) => feed,
        feed::FeedRead::Pending => None,
        feed::FeedRead::Unavailable(reason) => {
            ui.label(reason);
            None
        }
    };
    let present = present_relation(profiles, device, key).is_some();
    let (entries, note) = page_entries(feed.as_ref());
    let name = login::device_name(profiles, device, key);
    let chosen;
    let typed;
    let entered;
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let gap = state.clients.as_deref().and_then(client_gap);
        if let Some(note) = &note {
            ui.label(note);
        }
        if !present {
            ui.label(OFFLINE);
        }
        if let Some(gap) = &gap {
            ui.colored_label(egui::Color32::RED, gap);
        }
        let ready = state.clients.is_some() && state.ask.is_none() && state.login.entry.is_none();
        let enabled = present && !state.busy && gap.is_none() && ready;
        chosen = entry_buttons(ui, &entries, enabled);
        login::status(ui, &shared, &mut state, (device, key), present);
        typed = credentials_form(ui, &mut state);
        entered = login::entry_dialog(ui, &mut state.login, &name);
        if state.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(OPENING);
            });
        }
        if let Some(info) = &state.info {
            ui.label(info);
        }
        if let Some(error) = &state.error {
            let text = super::device_page_layout::explain_remote_error(error);
            ui.colored_label(egui::Color32::RED, text);
        }
    }
    let start = |remote_app| Start {
        device: device.to_string(),
        key: key.to_string(),
        remote_app,
        credentials: None,
        password: None,
        ask_password: true,
    };
    let session = match (chosen, typed, entered) {
        (Some(remote_app), _, _) => Some(start(remote_app)),
        (None, Some((remote_app, credentials)), _) => Some(Start {
            credentials: Some(credentials),
            ..start(remote_app)
        }),
        (None, None, Some((remote_app, account, password))) => Some(Start {
            ask_password: password.is_some(),
            password: password.map(|password| (account, password)),
            ..start(remote_app)
        }),
        (None, None, None) => None,
    };
    if let Some(session) = session {
        open(ui.ctx(), &shared, session, owned_nodes.clone());
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
    let nodes: Vec<&str> = relation_nodes(profiles, device, key)
        .into_iter()
        .chain(observed.iter().map(String::as_str))
        .filter(|node| !node.is_empty())
        .collect();
    sessions::show(ui, &nodes);
}

/// Every node this device may use under its stored relations; the session
/// list shows the sessions of these nodes.
pub fn relation_nodes<'a>(profiles: &'a ShareProfiles, device: &str, key: &str) -> Vec<&'a str> {
    let direct = profiles
        .direct_contacts
        .iter()
        .filter(|c| {
            c.remote_device_id.as_deref() == Some(device)
                && c.remote_public_key.as_deref() == Some(key)
        })
        .flat_map(|c| {
            [
                c.expected_node_id.as_str(),
                c.presence.as_ref().map_or("", |p| p.node_id.as_str()),
            ]
        });
    let rooms = profiles
        .rooms
        .iter()
        .flat_map(|room| &room.members)
        .filter(|m| m.device_id == device && m.public_key == key && !m.blocked)
        .map(|m| m.node_id.as_str());
    let grants = profiles
        .direct_grants
        .iter()
        .filter(|g| g.device_id == device && g.public_key == key)
        .map(|g| g.node_id.as_str());
    direct.chain(rooms).chain(grants).collect()
}

/// The locally installed clients, read once per page in the background.
fn read_clients(ctx: &egui::Context, shared: &Shared) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        if state.clients.is_some() || state.clients_reading {
            return;
        }
        state.clients_reading = true;
    }
    let (state, context) = (shared.clone(), ctx.clone());
    if let Err(error) = std::thread::Builder::new()
        .name("device-rdp-clients".into())
        .spawn(move || {
            let clients = crate::rdp::local_clients();
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.clients = Some(clients);
            state.clients_reading = false;
            context.request_repaint();
        })
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.clients = Some(Vec::new());
        state.clients_reading = false;
        state.error = Some(format!("RDP-Programme nicht ermittelt: {error}"));
    }
}

/// The account form for a FreeRDP client that cannot ask without a
/// terminal (design 2.8); returns the start with the typed credentials. The
/// password goes only to the client's standard input, never to argv, files
/// or logs, and leaves this form when it is sent.
fn credentials_form(
    ui: &mut egui::Ui,
    state: &mut State,
) -> Option<(Option<String>, crate::rdp::Credentials)> {
    let remote_app = state.ask.clone()?;
    ui.label(CREDENTIALS);
    egui::Grid::new("rdp-credentials")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Benutzer");
            ui.text_edit_singleline(&mut state.user);
            ui.end_row();
            ui.label("Domäne (optional)");
            ui.text_edit_singleline(&mut state.domain);
            ui.end_row();
            ui.label("Kennwort");
            ui.add(egui::TextEdit::singleline(&mut state.password).password(true));
            ui.end_row();
        });
    let mut typed = None;
    ui.horizontal(|ui| {
        let complete = !state.user.trim().is_empty() && !state.password.is_empty();
        if ui
            .add_enabled(complete, egui::Button::new("Verbinden"))
            .clicked()
        {
            let password = std::mem::take(&mut state.password);
            let domain = state.domain.trim();
            typed = Some((
                remote_app.clone(),
                crate::rdp::Credentials {
                    username: state.user.trim().to_string(),
                    domain: (!domain.is_empty()).then(|| domain.to_string()),
                    password: crate::share::apps_wire::ManagedSecret::new(password),
                },
            ));
            state.ask = None;
        }
        if ui.button("Abbrechen").clicked() {
            state.password.clear();
            state.ask = None;
        }
    });
    typed
}

/// Shown above the account form (design 2.8).
const CREDENTIALS: &str = "Das RDP-Programm dieses Rechners fragt ohne Terminal nicht selbst nach der Anmeldung (mit dem Paket freerdp3-sdl schon). Benutzer und Kennwort des Kontos auf dem Gerät:";

/// One start of the device page.
struct Start {
    device: String,
    key: String,
    remote_app: Option<String>,
    credentials: Option<crate::rdp::Credentials>,
    /// P4: the account and password typed into the dialog; the device
    /// checks and stores them before the start.
    password: Option<(String, crate::share::apps_wire::ManagedSecret)>,
    /// P4: a device that needs the password once ends the start with the
    /// dialog; `false` after „Ohne Speichern verbinden“ (the RDP program asks).
    ask_password: bool,
}

/// One click: relation, probe, host setup, service, client, trust and start
/// all happen in `rdp::open_device_with`; the session goes to this
/// process's owner registry, which ends the client with the window (design
/// 2.1). A FreeRDP client without terminal asks through the page's own
/// account form instead (design 2.8).
fn open(ctx: &egui::Context, shared: &Shared, start: Start, owned_nodes: OwnedNodes) {
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = true;
        state.error = None;
        state.info = None;
    }
    let (state, context) = (shared.clone(), ctx.clone());
    if let Err(error) = std::thread::Builder::new()
        .name("device-rdp-open".into())
        .spawn(move || {
            if let Some((account, password)) = start.password {
                let stored = crate::rdp::viewer_login::store_password(
                    &start.device,
                    &start.key,
                    Some(password),
                    None,
                );
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                match stored {
                    Ok(info) => state.login.note(&info),
                    Err(error) => {
                        // Wrong password or refused: the dialog again.
                        state.busy = false;
                        state.error = Some(error);
                        state.login.entry = Some((start.remote_app, account));
                        context.request_repaint();
                        return;
                    }
                }
            }
            let typed = start.credentials.is_some();
            let options = crate::rdp::OpenOptions {
                remote_app: start.remote_app.clone(),
                credentials: start.credentials,
                ask_password: start.ask_password,
                ..Default::default()
            };
            let result = crate::rdp::open_device_with(&start.device, &start.key, options, || {
                context.request_repaint();
            });
            // The node comes from the admitted, authenticated tunnel receipt.
            if let Ok(session) = &result {
                owned_nodes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(session.tunnel.remote_node.clone());
            }
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            state.busy = false;
            match result {
                Ok(session) => {
                    if let Some(login) = &session.login {
                        state.login.note_view(login);
                    }
                    let login = session.login.as_ref().map(|login| login.text.as_str());
                    state.info = Some(format!(
                        "{} läuft. {} {}",
                        session.remote_app.as_deref().unwrap_or(tiles::RDP_LABEL),
                        session.session_notice,
                        login.unwrap_or_default()
                    ))
                }
                Err(error) if crate::rdp::core::entry_needed_account(&error).is_some() => {
                    let account = crate::rdp::core::entry_needed_account(&error)
                        .unwrap_or_default()
                        .to_string();
                    state.login.entry = Some((start.remote_app, account));
                }
                Err(error) if !typed && error.contains(crate::rdp::NEEDS_SDL) => {
                    state.ask = Some(start.remote_app);
                }
                Err(error) => state.error = Some(error),
            }
            context.request_repaint();
        })
    {
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.busy = false;
        state.error = Some(error.to_string());
    }
}

#[cfg(test)]
#[path = "einrichtung_ui_task_tests.rs"]
mod einrichtung_ui_task_tests;
