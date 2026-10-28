//! The device feed on se Desktop tiles, the device pages and the CLI
//! (design 5.2, 4.3, 2.5; user rule r5 „wenn n fenster offen ist sehen es
//! andere se desktops … wenn auf localhost was offen ist wird das frei
//! gegeben für andere geräte. per default“): the open windows, the locally
//! listening services with their address on this device, and the Remote
//! Desktop handlings of one device. Every surface reads this one
//! projection of the background service's feed, so texts and handlings
//! never differ between surfaces.
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::rdp::core::{RdpFeedSummary, SetupState};
use crate::screen::core::family_protocol::FamilyListing;
use crate::share::loopback_services::LoopbackService;

/// The one handling for the whole desktop (design 2.1, user rule r3).
pub(crate) const RDP_LABEL: &str = "Remote Desktop";
/// The host reports Remote Desktop as set up but offers no startable
/// service; the handling runs the host's setup again (trigger T4).
pub(crate) const RDP_NOT_STARTABLE: &str =
    "Das Gerät meldet keinen startbaren Remote-Desktop-Dienst; „Remote Desktop“ richtet ihn dort erneut ein.";
/// Shown while no service list arrived from a device with steering rights.
pub(crate) const SERVICES_NONE: &str = "Auf diesem Gerät lauscht kein freigegebener Dienst.";

/// One Remote Desktop handling: the whole desktop or one published
/// RemoteApp (design 2.5). One click starts exactly this entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RdpEntry {
    pub(crate) label: String,
    /// `None`: the whole desktop.
    pub(crate) remote_app: Option<String>,
}

/// One open window of the device (design 5.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TileWindow {
    pub(crate) title: String,
    pub(crate) app: String,
    /// Exact `window:@family…` selector of the authenticated listing.
    pub(crate) source: String,
}

/// One locally listening service of the device and its address here
/// (design 4.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TileService {
    /// The port on the device.
    pub(crate) port: u16,
    pub(crate) process: Option<String>,
    /// The device address on this device; `None` while the background
    /// service has not bound it yet.
    pub(crate) address: Option<SocketAddr>,
    /// `http://` (`https://` for 443) for the usual web ports.
    pub(crate) url: Option<String>,
}

impl TileService {
    /// „<Prozess> – http://127.x.y.z:Port“ (design 4.3), with the substitute
    /// port spelled out when the device's port was taken here.
    pub(crate) fn line(&self) -> String {
        let port = self.port;
        let name = match &self.process {
            Some(process) => process.clone(),
            None => format!("Port {port}"),
        };
        let Some(address) = self.address else {
            return format!("{name} – Port {port} (Adresse wird eingerichtet)");
        };
        if address.port() != port {
            return format!("{name} – {port} → {address} (Port belegt)");
        }
        match &self.url {
            Some(url) => format!("{name} – {url}"),
            None => format!("{name} – {address}"),
        }
    }

    /// The text a click copies or opens.
    pub(crate) fn target(&self) -> Option<String> {
        self.url
            .clone()
            .or_else(|| self.address.map(|address| address.to_string()))
    }
}

/// The feed of one device as every surface shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TileFeed {
    /// A feed exists for this device (the device is present and new enough).
    #[serde(default)]
    pub(crate) known: bool,
    #[serde(default)]
    pub(crate) windows: Vec<TileWindow>,
    #[serde(default)]
    pub(crate) services: Vec<TileService>,
    /// RDP short state; present only for a device that lets this one steer.
    #[serde(default)]
    pub(crate) rdp: Option<RdpFeedSummary>,
    /// Why windows, services or the whole feed are unavailable.
    #[serde(default)]
    pub(crate) notes: Vec<String>,
    /// The device lets this one steer its screen: it sends services and the
    /// RDP state only then (design 5.2). Windows open with control.
    #[serde(default)]
    pub(crate) control: bool,
}

impl TileFeed {
    /// The Remote Desktop entries of a tile menu: only when the RDP short
    /// state says startable or setup possible (design 5.2).
    pub(crate) fn tile_rdp_entries(&self) -> Vec<RdpEntry> {
        match &self.rdp {
            Some(rdp) => rdp_entries(Some(rdp)).0,
            None => Vec::new(),
        }
    }

    /// „3 Fenster · 2 Dienste“ for the tile's status line; `None` when empty.
    pub(crate) fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        match self.windows.len() {
            0 => {}
            1 => parts.push("1 Fenster".to_string()),
            count => parts.push(format!("{count} Fenster")),
        }
        match self.services.len() {
            0 => {}
            1 => parts.push("1 Dienst".to_string()),
            count => parts.push(format!("{count} Dienste")),
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// The Remote Desktop handlings for an RDP short state and the state text
/// beside them (design 2.1, 2.5, 3.4). Without a short state (no feed yet,
/// an older device, no steering right) the one handling is offered: its
/// click probes the device and reports the real outcome. An impossible
/// setup offers no handling, only its reason.
pub(crate) fn rdp_entries(rdp: Option<&RdpFeedSummary>) -> (Vec<RdpEntry>, Option<String>) {
    let whole = RdpEntry {
        label: RDP_LABEL.into(),
        remote_app: None,
    };
    let Some(rdp) = rdp else {
        return (vec![whole], None);
    };
    if !rdp.startable && !rdp.setup.setup_possible() {
        return (Vec::new(), Some(rdp.setup.text()));
    }
    let mut entries = vec![whole];
    if rdp.startable {
        entries.extend(rdp.remote_apps.iter().map(|app| RdpEntry {
            label: format!("{} (RemoteApp)", app.display_name),
            remote_app: Some(app.alias.clone()),
        }));
    }
    let note = match &rdp.setup {
        _ if rdp.startable => None,
        SetupState::Done | SetupState::NotNeeded => Some(RDP_NOT_STARTABLE.into()),
        other => Some(other.text()),
    };
    (entries, note)
}

/// `http://` for 80, 3000–3999, 5000–5999 and 8000–8999, `https://` for 443
/// (design 4.3); other ports are copied as `address:port`.
pub(crate) fn service_url(address: SocketAddr) -> Option<String> {
    match address.port() {
        443 => Some(format!("https://{address}")),
        80 | 3000..=3999 | 5000..=5999 | 8000..=8999 => Some(format!("http://{address}")),
        _ => None,
    }
}

/// The facts of one device's feed as the background service holds them.
pub(crate) struct FeedFacts<'a> {
    pub(crate) families: Option<&'a FamilyListing>,
    pub(crate) families_unavailable: Option<&'a str>,
    pub(crate) services: Option<&'a [LoopbackService]>,
    pub(crate) services_unavailable: Option<&'a str>,
    pub(crate) rdp: Option<&'a RdpFeedSummary>,
    pub(crate) unavailable: Option<&'a str>,
}

/// Projects one feed; `mapped(port)` is the address the background service
/// bound for that device port here (design 4.3).
pub(crate) fn project_feed(
    facts: FeedFacts<'_>,
    mapped: impl Fn(u16) -> Option<SocketAddr>,
) -> TileFeed {
    let windows: Vec<TileWindow> = facts
        .families
        .map(|listing| {
            listing
                .entries
                .iter()
                .map(|entry| TileWindow {
                    title: entry.window.title.clone(),
                    app: entry.window.app.clone(),
                    source: entry.source.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let services: Vec<TileService> = facts
        .services
        .unwrap_or_default()
        .iter()
        .map(|service| {
            let address = mapped(service.port);
            TileService {
                port: service.port,
                process: service.process.clone(),
                address,
                url: address.and_then(service_url),
            }
        })
        .collect();
    let mut notes: Vec<String> = [
        facts.unavailable,
        facts.families_unavailable,
        facts.services_unavailable,
    ]
    .into_iter()
    .flatten()
    .map(str::to_string)
    .collect();
    if facts.services.is_some_and(|services| services.is_empty()) {
        notes.push(SERVICES_NONE.into());
    }
    if let Some(listing) = facts.families {
        notes.extend(listing.limitations.iter().cloned());
    }
    TileFeed {
        known: facts.unavailable.is_none(),
        windows,
        services,
        rdp: facts.rdp.cloned(),
        notes,
        control: facts.rdp.is_some() || facts.services.is_some(),
    }
}

/// The feed of the device `device_id`/`public_key` in a snapshot of the
/// background service; `None` while it holds none for that device. The
/// device sends services and the RDP state only to a device it lets steer
/// (design 5.2), so an empty service list without RDP state means „no
/// steering right“, not „nothing listens“.
pub(crate) fn feed_of(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    device_id: &str,
    public_key: &str,
) -> Option<TileFeed> {
    let view = snapshot
        .peer_feeds
        .iter()
        .find(|view| view.device_id == device_id && view.public_key == public_key)?;
    let steers = view.rdp.is_some() || !view.services.is_empty();
    let facts = FeedFacts {
        families: view.families.as_ref(),
        families_unavailable: view.families_unavailable.as_deref(),
        services: steers.then_some(view.services.as_slice()),
        services_unavailable: view.services_unavailable.as_deref(),
        rdp: view.rdp.as_ref(),
        unavailable: view.unavailable.as_deref(),
    };
    let mapped = |port: u16| {
        view.mapped
            .iter()
            .find(|mapping| mapping.host_port == port)
            .and_then(|mapping| mapping.address)
    };
    let mut feed = project_feed(facts, mapped);
    feed.notes.extend(view.mapped.iter().filter_map(|mapping| {
        let error = mapping.last_error.as_ref()?;
        Some(format!("Port {}: {error}", mapping.host_port))
    }));
    Some(feed)
}

/// The authenticated window listing of that device's running feed (the
/// device page opens an exact entry of it); `None` once the feed ended, so
/// CLI and MCP ask the device itself instead of serving an old list.
pub(crate) fn families_of(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    device_id: &str,
    public_key: &str,
) -> Option<FamilyListing> {
    snapshot
        .peer_feeds
        .iter()
        .find(|view| view.device_id == device_id && view.public_key == public_key)
        .filter(|view| view.live)?
        .families
        .clone()
}

/// The device identity (device ID, public key) a relation reaches.
pub(crate) fn identity_of(
    profiles: &crate::share::ShareProfiles,
    target: &crate::share::PeerOpenTarget,
) -> Option<(String, String)> {
    match target {
        crate::share::PeerOpenTarget::Direct { contact_id } => {
            let contact = profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == *contact_id)?;
            Some((
                contact.remote_device_id.clone()?,
                contact.remote_public_key.clone()?,
            ))
        }
        crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => profiles
            .rooms
            .iter()
            .filter(|room| room.room_id == *room_id || room.id == *room_id)
            .flat_map(|room| &room.members)
            .find(|member| member.device_id == *device_id && !member.blocked)
            .map(|member| (member.device_id.clone(), member.public_key.clone())),
    }
}
