//! Bounded complete snapshots; never silently truncate a window family.
use std::collections::HashSet;
use std::io;
use super::model::*;

pub(crate) const MAX_FAMILY_BYTES: usize = 1024 * 1024;
fn invalid(text: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, text) }

impl FamilySnapshot {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.version != FAMILY_VERSION || self.session.len() != 32
            || !self.session.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.revision == 0 || self.layout_revision==0 || self.layout_revision>self.revision || self.root.session != self.session {
            return Err(invalid("invalid_window_family_snapshot"));
        }
        let mut identities = HashSet::new();
        for member in &self.members {
            if member.identity.session != self.session || member.identity.generation == 0
                || !identities.insert(&member.identity) || !member.geometry.valid() {
                return Err(invalid("invalid_window_family_member"));
            }
            if member.app_id.as_deref().is_some_and(|id| !crate::screen::core::app_icon::valid_app_id(id)) {
                return Err(invalid("invalid_window_family_app_identity"));
            }
            if let Some(icon) = &member.icon {
                icon.validate()?;
                if member.app_id.as_deref() != Some(icon.app_id.as_str()) {
                    return Err(invalid("window_family_icon_app_mismatch"));
                }
            }
        }
        if !identities.contains(&self.root) { return Err(invalid("window_family_root_missing")); }
        for member in &self.members {
            if member.identity == self.root {
                if member.owner.is_some() { return Err(invalid("window_family_root_has_owner")); }
                continue;
            }
            let mut current = member;
            let mut visited = HashSet::new();
            while current.identity != self.root {
                if !visited.insert(&current.identity) { return Err(invalid("window_family_owner_cycle")); }
                let owner = current.owner.as_ref().ok_or_else(|| invalid("window_family_owner_missing"))?;
                current = self.members.iter().find(|member| &member.identity == owner)
                    .ok_or_else(|| invalid("window_family_owner_outside_snapshot"))?;
            }
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(invalid_json)?;
        if bytes.len() > MAX_FAMILY_BYTES { return Err(invalid("window_family_exceeds_frame")); }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_FAMILY_BYTES { return Err(invalid("window_family_exceeds_frame")); }
        let snapshot: Self = serde_json::from_slice(bytes).map_err(invalid_json)?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn invalid_json(error: serde_json::Error) -> io::Error { invalid(&format!("window_family_json: {error}")) }
