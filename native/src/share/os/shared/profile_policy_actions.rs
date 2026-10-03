//! Persisted FC1 rights edits shared by CLI and Mobile.
use super::direct_protocol::DirectPeerIdentity;
use super::direct_relation_actions::RelationChange;
use super::profiles::ShareProfiles;

pub fn set_direct_peer_write(
    default_home: Option<String>,
    expected: &DirectPeerIdentity,
    write: bool,
) -> Result<RelationChange, String> {
    let now = super::core::now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home, |profiles| {
        changed = profiles.set_direct_peer_write(expected, write, now)?;
        Ok(())
    })?;
    Ok(RelationChange { profiles, changed })
}
