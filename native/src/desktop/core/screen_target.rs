//! Start argument of the S4 viewer (C4.9, V5): `smart_explorer --screen-view
//! <ziel-json>` with `{"relation","relation_id","device_id","source","control"}`.
//! `source: null` asks for the whole screen (Annahme A28). se Desktop embeds
//! no viewer; it only starts this process (Ä17).
use serde::{Deserialize, Serialize};

use super::tiles::DeviceTile;
use crate::share::PeerOpenTarget;

/// The S4 target JSON; field order as in V5.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScreenTarget {
    /// `direct` or `room`.
    pub(crate) relation: String,
    /// Contact ID (Direct) or local room profile ID (Room).
    pub(crate) relation_id: String,
    pub(crate) device_id: String,
    /// `null`: the whole screen.
    pub(crate) source: Option<String>,
    /// `true` for "Bildschirm steuern", `false` for "Bildschirm ansehen".
    pub(crate) control: bool,
    /// Authenticated node pin for app receipts; ordinary screen routes omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expected_node: Option<String>,
}

impl ScreenTarget {
    pub(crate) fn new(target: &PeerOpenTarget, device_id: &str, control: bool) -> ScreenTarget {
        let (relation, relation_id) = match target {
            PeerOpenTarget::Direct { contact_id } => ("direct", contact_id.clone()),
            PeerOpenTarget::RoomDevice { room_id, .. } => ("room", room_id.clone()),
        };
        ScreenTarget {
            expected_node: None,
            relation: relation.into(),
            relation_id,
            device_id: device_id.into(),
            source: None,
            control,
        }
    }

    /// The target of a tile; `None` while a Direct peer's device is unknown
    /// (then the menu offers no screen).
    pub(crate) fn for_tile(tile: &DeviceTile, control: bool) -> Option<ScreenTarget> {
        let device_id = tile.device_id.as_deref()?;
        Some(ScreenTarget::new(&tile.target, device_id, control))
    }

    /// The value after `--screen-view`.
    pub(crate) fn to_arg(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}
