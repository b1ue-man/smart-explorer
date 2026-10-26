//! P4 extension version 1. Native pins never cross the peer protocol.
use super::input_model::{encode_batch, InputEvent};
use super::window_list::{SourceId, WindowEntry};
use crate::remote_window::{
    FamilySnapshot, OperationStatus, WindowIdentity, WindowInputTarget, WindowOperation,
};
use serde::{Deserialize, Serialize};
use std::io;

pub(crate) const FAMILY_PROTOCOL: u16 = 1;
// '@' cannot begin an Android package ID; Windows/X11 IDs are decimal and
// the existing Wayland IDs are handles/UUIDs. Keep their source meanings intact.
pub(crate) const FAMILY_PREFIX: &str = "@family.";

pub(crate) fn token() -> io::Result<String> {
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(nonce.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn selector(source: &SourceId) -> Option<&str> {
    let SourceId::Window(id) = source else {
        return None;
    };
    id.strip_prefix(FAMILY_PREFIX).filter(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub(crate) fn owner(
    principal: &crate::share::relation_proof::ProvenPrincipal,
) -> io::Result<String> {
    super::wire::to_json(&(
        &principal.relation_kind,
        &principal.relation_id,
        &principal.device_id,
        &principal.node_id,
        &principal.public_key,
    ))
    .and_then(|bytes| String::from_utf8(bytes).map_err(io::Error::other))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyListEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_request: Option<String>,
    pub source: SourceId,
    pub identity: WindowIdentity,
    pub window: WindowEntry,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyListing {
    pub version: u16,
    pub entries: Vec<FamilyListEntry>,
    pub limitations: Vec<String>,
}
impl FamilyListing {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.version != FAMILY_PROTOCOL {
            return Err(super::wire::invalid("unsupported window family contract"));
        }
        let mut ids = std::collections::HashSet::new();
        for entry in &self.entries {
            if selector(&entry.source) != Some(entry.identity.session.as_str())
                || entry.identity.generation != 1
                || !ids.insert(&entry.identity)
            {
                return Err(super::wire::invalid("invalid pinned family listing"));
            }
        }
        if super::wire::to_json(self)?.len() > super::wire::MAX_WINDOWS_BYTES {
            return Err(super::wire::invalid("pinned family listing exceeds frame"));
        }
        Ok(())
    }
}

impl FamilySnapshot {
    /// Coordinates are normalized over the composed family canvas. Returned
    /// coordinates are normalized over the selected member for FAMILY_INPUT.
    pub(crate) fn pointer_target(&self, x: u16, y: u16) -> Option<(WindowInputTarget, u16, u16)> {
        let visible: Vec<_> = self
            .members
            .iter()
            .filter(|m| m.visible && !m.minimized)
            .collect();
        let left = visible.iter().map(|m| m.geometry.x).min()?;
        let top = visible.iter().map(|m| m.geometry.y).min()?;
        let right = visible
            .iter()
            .map(|m| i64::from(m.geometry.x) + i64::from(m.geometry.width))
            .max()?;
        let bottom = visible
            .iter()
            .map(|m| i64::from(m.geometry.y) + i64::from(m.geometry.height))
            .max()?;
        let px = i64::from(left) + i64::from(x) * (right - i64::from(left) - 1).max(0) / 65_535;
        let py = i64::from(top) + i64::from(y) * (bottom - i64::from(top) - 1).max(0) / 65_535;
        let (px, py) = (i32::try_from(px).ok()?, i32::try_from(py).ok()?);
        let member = visible
            .into_iter()
            .filter(|m| m.operations.input && m.geometry.contains(px, py))
            .max_by_key(|m| m.z_order)?;
        // The host maps member positions back to pixels by integer division.
        // Round this inverse upwards so a member pixel survives that round trip.
        let along = |p: i32, start: i32, length: u32| {
            let span = i64::from(length.saturating_sub(1).max(1));
            (((i64::from(p) - i64::from(start)) * 65_535 + span - 1) / span).clamp(0, 65_535) as u16
        };
        Some((
            WindowInputTarget {
                identity: member.identity.clone(),
                snapshot_revision: self.revision,
            },
            along(px, member.geometry.x, member.geometry.width),
            along(py, member.geometry.y, member.geometry.height),
        ))
    }
    pub(crate) fn focused_target(&self) -> Option<WindowInputTarget> {
        self.members
            .iter()
            .filter(|m| m.focused && m.visible && !m.minimized && m.operations.input)
            .max_by_key(|m| m.z_order)
            .map(|m| WindowInputTarget {
                identity: m.identity.clone(),
                snapshot_revision: self.revision,
            })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyOperation {
    pub request_id: String,
    pub identity: WindowIdentity,
    pub operation: WindowOperation,
}
impl FamilyOperation {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.request_id.chars().any(char::is_control)
        {
            return Err(super::wire::invalid(
                "invalid family operation request identity",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyOperationResult {
    pub request_id: String,
    pub status: Option<OperationStatus>,
    pub reason: Option<String>,
    pub snapshot: Option<FamilySnapshot>,
}
impl FamilyOperationResult {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.request_id.chars().any(char::is_control)
            || self.status.is_some() == self.reason.is_some()
        {
            return Err(super::wire::invalid("invalid family operation result"));
        }
        if let Some(snapshot) = &self.snapshot {
            snapshot.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyInput {
    pub target: WindowInputTarget,
    pub events: Vec<InputEvent>,
}
impl FamilyInput {
    pub(crate) fn validate(&self) -> io::Result<()> {
        encode_batch(&self.events).map(|_| ())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FamilyInputResult {
    pub blocked: Option<super::wire_control::ControlBlock>,
    pub reason: Option<String>,
}
