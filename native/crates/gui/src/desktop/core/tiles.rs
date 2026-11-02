//! Tiles of se Desktop (C4.4–C4.6): devices and rooms from the Share
//! profiles, with live states only while the background service runs (the
//! shell never starts it, V11), vaults from S3. Online rule and status codes
//! come from `share::device_view`, the same functions as the Android bridge.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::layout::ItemId;
use crate::share::device_view;

#[path = "tile_feed.rs"]
mod feed;
use crate::share::{
    DirectContact, PeerOpenTarget, RoomMember, RoomProfile, ShareProfiles, ShareStatus,
};
pub use feed::*;

/// Shown for every device while only the profiles are known (C4.5).
pub const UNKNOWN_TEXT: &str = "Status unbekannt – nicht verbunden";

/// Code of a device tile: the ten Share status codes plus `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TileCode {
    Unknown,
    Offline,
    Waiting,
    WaitingForAccess,
    Available,
    Connecting,
    Connected,
    ConnectedDirect,
    ConnectedRelay,
    Failed,
    IdentityConflict,
}

impl TileCode {
    pub fn from_status(status: &ShareStatus) -> TileCode {
        match status {
            ShareStatus::Offline => TileCode::Offline,
            ShareStatus::Waiting => TileCode::Waiting,
            ShareStatus::WaitingForAccess => TileCode::WaitingForAccess,
            ShareStatus::Available => TileCode::Available,
            ShareStatus::Connecting => TileCode::Connecting,
            ShareStatus::Connected => TileCode::Connected,
            ShareStatus::ConnectedDirect => TileCode::ConnectedDirect,
            ShareStatus::ConnectedRelay => TileCode::ConnectedRelay,
            ShareStatus::Failed(_) => TileCode::Failed,
            ShareStatus::IdentityConflict => TileCode::IdentityConflict,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TileCode::Unknown => "unknown",
            TileCode::Offline => "offline",
            TileCode::Waiting => "waiting",
            TileCode::WaitingForAccess => "waitingForAccess",
            TileCode::Available => "available",
            TileCode::Connecting => "connecting",
            TileCode::Connected => "connected",
            TileCode::ConnectedDirect => "connectedDirect",
            TileCode::ConnectedRelay => "connectedRelay",
            TileCode::Failed => "failed",
            TileCode::IdentityConflict => "identityConflict",
        }
    }

    /// C4.5: a double click opens the device's files, except while the peer
    /// has not granted access yet or its identity changed (then the page).
    pub fn opens_files(&self) -> bool {
        !matches!(
            self,
            TileCode::WaitingForAccess | TileCode::IdentityConflict
        )
    }
}

/// `share_state` of the status: live or only the stored profiles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareState {
    Live,
    Profiles { reason: String },
}

/// What a double click starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TileOpen {
    /// `--open-location <LOCATOR>`.
    Location(String),
    /// `--open-page <SEITE>`.
    Page(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTile {
    pub target: PeerOpenTarget,
    /// Key in the form of `mount::device_key` (V4): layout and document IDs.
    pub key: String,
    pub name: String,
    pub code: TileCode,
    /// Status text; for `failed` the error (tooltip).
    pub status_text: String,
    pub last_seen: Option<i64>,
    pub lan: bool,
    pub online: bool,
    /// Whether this device lets a Direct peer write (`share.status` `write`).
    pub write: Option<bool>,
    /// `share://direct/<id>/` or `share://room/<raum>/<gerät>/`.
    pub location: String,
    /// Device ID for the S4 viewer target; unknown for an unpaired Direct peer.
    pub device_id: Option<String>,
    /// Folder of the device in the S2 device tree while it is mounted.
    pub folder_path: Option<String>,
    pub open: TileOpen,
    /// Windows, services and Remote Desktop of the device from the
    /// background service's feed (design 5.2); empty without a feed.
    #[serde(default)]
    pub feed: TileFeed,
}

/// A room as folder group with its member devices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomGroup {
    /// Local room profile ID (as in `share://room/<raum>/…`).
    pub room_id: String,
    pub name: String,
    /// `--open-page share-room:<room_id>`.
    pub page: String,
    pub members: Vec<DeviceTile>,
}

/// A vault location from S3 `Vault::Status`: only ID, label and state (S5-V5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultTile {
    pub site: String,
    pub label: String,
    pub state: String,
}

impl VaultTile {
    /// The only vault locator se Desktop uses (S5-V4).
    pub fn location(&self) -> String {
        format!("vault://{}/", self.site)
    }
}

/// Tiles of the empty state (C4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyAction {
    AddDevice,
    JoinRoom,
}

impl EmptyAction {
    pub fn label(self) -> &'static str {
        match self {
            EmptyAction::AddDevice => "Gerät hinzufügen",
            EmptyAction::JoinRoom => "Raum beitreten",
        }
    }

    /// `--open-page` target.
    pub fn page(self) -> &'static str {
        match self {
            EmptyAction::AddDevice => "share-devices",
            EmptyAction::JoinRoom => "share-rooms",
        }
    }

    pub fn item_id(self) -> ItemId {
        match self {
            EmptyAction::AddDevice => ItemId::AddDevice,
            EmptyAction::JoinRoom => ItemId::JoinRoom,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileModel {
    pub share_state: ShareState,
    /// C4.6 "Profile nicht ladbar": the error line instead of tiles.
    pub profiles_error: Option<String>,
    pub devices: Vec<DeviceTile>,
    pub rooms: Vec<RoomGroup>,
    /// `None`: no vault group ("Tresore: Hintergrunddienst läuft nicht").
    pub vaults: Option<Vec<VaultTile>>,
    pub empty_actions: Vec<EmptyAction>,
}

/// Inputs of one projection, all read by the shell without starting anything.
pub struct TileInput<'a> {
    pub profiles: Result<&'a ShareProfiles, &'a str>,
    pub share_state: &'a ShareState,
    pub now_secs: i64,
    /// Device key → folder path from S2 `DevicesStatus` while mounted.
    pub device_paths: &'a BTreeMap<String, String>,
    /// S3 vault locations; `None` while the background service does not run.
    pub vaults: Option<&'a [VaultTile]>,
}

/// Key of a device as `mount::device_key` builds it (V4): `direct~<contact_id>`
/// or `room~<room_id>~<device_id>`, every byte outside `[A-Za-z0-9._-]` as
/// `%XX`. Since DRV-M6 this is the function of the device tree itself, so the
/// tiles of se Desktop and the folders of the tree can never differ.
pub fn device_key(target: &PeerOpenTarget) -> String {
    crate::mount::device_key(target)
}

/// Live: the code of the status. Profiles only: `unknown`, except a stored
/// identity conflict, which stays visible and never opens files.
fn tile_code(status: &ShareStatus, live: bool) -> TileCode {
    if live || *status == ShareStatus::IdentityConflict {
        TileCode::from_status(status)
    } else {
        TileCode::Unknown
    }
}

/// Facts of one device before projection.
struct DeviceFacts<'a> {
    target: PeerOpenTarget,
    name: &'a str,
    status: &'a ShareStatus,
    last_seen: Option<i64>,
    lan: bool,
    online: bool,
    write: Option<bool>,
    device_id: Option<String>,
    page: String,
}

fn device_tile(facts: DeviceFacts<'_>, live: bool, paths: &BTreeMap<String, String>) -> DeviceTile {
    let code = tile_code(facts.status, live);
    let key = device_key(&facts.target);
    let location = format!("{}/", facts.target.endpoint_prefix());
    let open = if code.opens_files() {
        TileOpen::Location(location.clone())
    } else {
        TileOpen::Page(facts.page)
    };
    DeviceTile {
        folder_path: paths.get(&key).cloned(),
        status_text: if code == TileCode::Unknown {
            UNKNOWN_TEXT.into()
        } else {
            facts.status.label()
        },
        target: facts.target,
        key,
        name: facts.name.to_string(),
        code,
        last_seen: facts.last_seen,
        lan: live && facts.lan,
        online: live && facts.online,
        write: facts.write,
        location,
        device_id: facts.device_id,
        open,
        feed: TileFeed::default(),
    }
}

fn direct_tile(
    profiles: &ShareProfiles,
    contact: &DirectContact,
    input: &TileInput<'_>,
) -> DeviceTile {
    let facts = DeviceFacts {
        target: PeerOpenTarget::Direct {
            contact_id: contact.id.clone(),
        },
        name: &contact.display_name,
        status: &contact.status,
        last_seen: contact.last_seen,
        lan: device_view::seen_on_lan(contact, input.now_secs),
        online: device_view::online(contact, input.now_secs),
        write: device_view::granted_write(profiles, contact),
        device_id: contact.remote_device_id.clone(),
        page: format!("share-device:{}", contact.id),
    };
    device_tile(facts, is_live(input), input.device_paths)
}

fn member_tile(room: &RoomProfile, member: &RoomMember, input: &TileInput<'_>) -> DeviceTile {
    let facts = DeviceFacts {
        target: PeerOpenTarget::RoomDevice {
            room_id: room.id.clone(),
            device_id: member.device_id.clone(),
        },
        name: &member.device_name,
        status: &member.status,
        last_seen: member.last_seen,
        lan: false,
        online: device_view::reachable(&member.status),
        write: None,
        device_id: Some(member.device_id.clone()),
        page: format!("share-room:{}", room.id),
    };
    device_tile(facts, is_live(input), input.device_paths)
}

fn is_live(input: &TileInput<'_>) -> bool {
    *input.share_state == ShareState::Live
}

/// The tiles of the surface for one model revision.
pub fn project(input: &TileInput<'_>) -> TileModel {
    let vaults = input.vaults.map(|vaults| vaults.to_vec());
    let profiles = match input.profiles {
        Ok(profiles) => profiles,
        Err(error) => {
            return TileModel {
                share_state: input.share_state.clone(),
                profiles_error: Some(error.to_string()),
                devices: Vec::new(),
                rooms: Vec::new(),
                vaults,
                empty_actions: Vec::new(),
            }
        }
    };
    let devices: Vec<DeviceTile> = profiles
        .direct_contacts
        .iter()
        .map(|contact| direct_tile(profiles, contact, input))
        .collect();
    let rooms: Vec<RoomGroup> = profiles
        .rooms
        .iter()
        .map(|room| RoomGroup {
            room_id: room.id.clone(),
            name: room.name.clone(),
            page: format!("share-room:{}", room.id),
            members: room
                .members
                .iter()
                .map(|member| member_tile(room, member, input))
                .collect(),
        })
        .collect();
    let empty_actions = if devices.is_empty() && rooms.is_empty() {
        vec![EmptyAction::AddDevice, EmptyAction::JoinRoom]
    } else {
        Vec::new()
    };
    TileModel {
        share_state: input.share_state.clone(),
        profiles_error: None,
        devices,
        rooms,
        vaults,
        empty_actions,
    }
}

/// Puts each device's feed on its tile (design 5.2); `lookup(device_id,
/// public_key)` reads the background service's feed of that device.
pub fn attach_feeds(
    model: &mut TileModel,
    profiles: &ShareProfiles,
    lookup: impl Fn(&str, &str) -> Option<TileFeed>,
) {
    let tiles = model.devices.iter_mut().chain(
        model
            .rooms
            .iter_mut()
            .flat_map(|room| room.members.iter_mut()),
    );
    for tile in tiles {
        tile.feed = identity_of(profiles, &tile.target)
            .and_then(|(device, key)| lookup(&device, &key))
            .unwrap_or_default();
    }
}
