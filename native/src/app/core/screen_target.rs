//! The target of the desktop viewer (SCR-M7, U25-2): the JSON after
//! `smart_explorer --screen-view` is `{"relation","relation_id","device_id",
//! "source","control"}` (contract V5; `fullscreen` is an optional addition of
//! `se screen view --fullscreen`, because the start parser allows exactly
//! one value). `relation` is `direct` (a contact) or `room` (a member of a
//! room); `relation_id` is the contact ID or the local room profile ID, as in
//! `PeerOpenTarget`. A Direct device is named by its pinned `device_id`: the
//! viewer refuses a contact whose device changed since the target was made.
use serde::{Deserialize, Serialize};

use crate::screen::core::window_list::SourceId;
use crate::share::{PeerOpenTarget, ShareProfiles};

/// Text for a target whose Direct device is no longer the one it named.
pub(crate) const DEVICE_CHANGED: &str =
    "Das Gerät hat sich geändert. Bitte die Verbindung zum Gerät prüfen.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Relation {
    Direct,
    Room,
}

/// What the viewer opens.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ScreenTarget {
    pub relation: Relation,
    /// Contact ID (`direct`) or local room profile ID (`room`).
    pub relation_id: String,
    /// The device's ID: `remote_device_id` of the contact, or the room
    /// member's device ID (required for rooms).
    #[serde(default)]
    pub device_id: Option<String>,
    /// `None`: the whole (primary) screen.
    #[serde(default)]
    pub source: Option<SourceId>,
    /// Start with control („Bildschirm steuern“).
    #[serde(default)]
    pub control: bool,
    /// Start in full screen (`se screen view --fullscreen`).
    #[serde(default)]
    pub fullscreen: bool,
}

impl ScreenTarget {
    /// The target of a Direct contact.
    pub(crate) fn direct(contact_id: &str, device_id: Option<&str>) -> Self {
        Self {
            relation: Relation::Direct,
            relation_id: contact_id.into(),
            device_id: device_id.map(str::to_string),
            source: None,
            control: false,
            fullscreen: false,
        }
    }

    /// The target of a room member.
    pub(crate) fn room(room_profile_id: &str, device_id: &str) -> Self {
        Self {
            relation: Relation::Room,
            relation_id: room_profile_id.into(),
            device_id: Some(device_id.into()),
            source: None,
            control: false,
            fullscreen: false,
        }
    }

    /// Reads the JSON of `--screen-view`.
    pub(crate) fn parse(json: &str) -> Result<Self, String> {
        let target: Self = serde_json::from_str(json)
            .map_err(|error| format!("Ungültiges Bildschirmziel: {error}"))?;
        if target.relation_id.trim().is_empty() {
            return Err("Ungültiges Bildschirmziel: relation_id fehlt".into());
        }
        if target.relation == Relation::Room
            && target.device_id.as_deref().is_none_or(str::is_empty)
        {
            return Err("Das Raumgerät fehlt im Bildschirmziel".into());
        }
        Ok(target)
    }

    /// The device the session connects to.
    pub(crate) fn to_peer(&self) -> Result<PeerOpenTarget, String> {
        match self.relation {
            Relation::Direct => Ok(PeerOpenTarget::Direct {
                contact_id: self.relation_id.clone(),
            }),
            Relation::Room => match self.device_id.as_deref().filter(|id| !id.is_empty()) {
                Some(device_id) => Ok(PeerOpenTarget::RoomDevice {
                    room_id: self.relation_id.clone(),
                    device_id: device_id.into(),
                }),
                None => Err("Das Raumgerät fehlt im Bildschirmziel".into()),
            },
        }
    }

    /// The same device, whatever the source and control are.
    pub(crate) fn same_device(&self, other: &Self) -> bool {
        self.relation == other.relation
            && self.relation_id == other.relation_id
            && self.device_id == other.device_id
    }

    /// Checks the target against the current profiles (U25-2): a Direct
    /// contact whose pinned device is not the named one is refused, and a
    /// device that is gone from the profiles cannot be opened.
    pub(crate) fn check_profiles(&self, profiles: &ShareProfiles) -> Result<(), String> {
        match self.relation {
            Relation::Direct => {
                let contact = profiles
                    .direct_contacts
                    .iter()
                    .find(|contact| contact.id == self.relation_id)
                    .ok_or("Das Gerät ist nicht mehr in der Geräteliste.")?;
                match (&self.device_id, &contact.remote_device_id) {
                    (Some(named), Some(current)) if named == current => Ok(()),
                    (Some(_), _) => Err(DEVICE_CHANGED.into()),
                    (None, _) => Ok(()),
                }
            }
            Relation::Room => {
                let room = profiles
                    .rooms
                    .iter()
                    .find(|room| room.id == self.relation_id || room.room_id == self.relation_id)
                    .ok_or("Der Raum ist nicht mehr gespeichert.")?;
                let device = self.device_id.as_deref().unwrap_or_default();
                if room.members.iter().any(|member| member.device_id == device) {
                    Ok(())
                } else {
                    Err("Das Gerät gehört nicht mehr zum Raum.".into())
                }
            }
        }
    }

    /// The name the profiles give the device (window title, device list).
    pub(crate) fn device_name(&self, profiles: &ShareProfiles) -> Option<String> {
        match self.relation {
            Relation::Direct => profiles
                .direct_contacts
                .iter()
                .find(|contact| contact.id == self.relation_id)
                .map(|contact| contact.display_name.clone()),
            Relation::Room => {
                let device = self.device_id.as_deref()?;
                profiles
                    .rooms
                    .iter()
                    .find(|room| room.id == self.relation_id || room.room_id == self.relation_id)?
                    .members
                    .iter()
                    .find(|member| member.device_id == device)
                    .map(|member| member.device_name.clone())
            }
        }
    }
}
