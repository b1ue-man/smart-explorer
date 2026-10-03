//! Persisted relation-rights actions without UI (FC1, FC5): „Wieder erlauben“
//! / „Bestätigen“ of a Direct grant and the share-back choice of a contact.
//! Each call commits one profile transaction and returns the committed
//! profiles, which the caller hands to the Share service.
use super::profiles::ShareProfiles;

/// A persisted relation change: the committed profiles and whether anything
/// changed (only then the service needs a new configuration).
pub struct RelationChange {
    pub profiles: ShareProfiles,
    pub changed: bool,
}

/// „Wieder erlauben“ for a blocked grant, „Bestätigen“ for one suspended
/// after a code rotation; Exec stays off.
pub fn allow_direct_peer_again(
    default_home: Option<String>,
    device_id: &str,
) -> Result<RelationChange, String> {
    let now = super::core::now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home, |profiles| {
        changed = profiles.allow_direct_grant_again(device_id, now)?;
        Ok(())
    })?;
    Ok(RelationChange { profiles, changed })
}

/// „Auch meine Freigaben für dieses Gerät öffnen“ for one contact.
pub fn set_direct_share_back(
    default_home: Option<String>,
    contact_id: &str,
    share_back: bool,
) -> Result<RelationChange, String> {
    let now = super::core::now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home, |profiles| {
        changed = profiles.set_contact_share_back(contact_id, share_back, now)?;
        Ok(())
    })?;
    Ok(RelationChange { profiles, changed })
}
