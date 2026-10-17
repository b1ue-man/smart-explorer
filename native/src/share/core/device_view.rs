//! How a Share device appears to the user, shared by the Android bridge
//! (`share.status`) and the se Desktop tiles (DSK-M1, C4.4), so both show the
//! same status code and the same online state for the same profiles.
use crate::share::{DirectContact, ShareProfiles, ShareStatus, LAN_PRESENCE_TTL_SECS};

/// Status code of a device (`status` of `share.status`, tile code).
pub(crate) fn status_code(status: &ShareStatus) -> &'static str {
    match status {
        ShareStatus::Offline => "offline",
        ShareStatus::Waiting => "waiting",
        ShareStatus::WaitingForAccess => "waitingForAccess",
        ShareStatus::Available => "available",
        ShareStatus::Connecting => "connecting",
        ShareStatus::Connected => "connected",
        ShareStatus::ConnectedDirect => "connectedDirect",
        ShareStatus::ConnectedRelay => "connectedRelay",
        ShareStatus::Failed(_) => "failed",
        ShareStatus::IdentityConflict => "identityConflict",
    }
}

/// The peer answers or a session runs.
pub(crate) fn reachable(status: &ShareStatus) -> bool {
    matches!(
        status,
        ShareStatus::Available
            | ShareStatus::Connected
            | ShareStatus::ConnectedDirect
            | ShareStatus::ConnectedRelay
    )
}

/// Seen on the local network within the presence lifetime, with an address.
pub(crate) fn seen_on_lan(contact: &DirectContact, now_secs: i64) -> bool {
    contact.lan_seen_at.is_some_and(|seen| {
        now_secs.saturating_sub(seen) <= LAN_PRESENCE_TTL_SECS && !contact.lan_candidates.is_empty()
    })
}

/// `online` of a Direct device: reachable or seen on the local network.
pub(crate) fn online(contact: &DirectContact, now_secs: i64) -> bool {
    reachable(&contact.status) || seen_on_lan(contact, now_secs)
}

/// Whether this device lets the contact write (`write` of `share.status`):
/// the single grant of exactly the contact's identity; `None` without a known
/// identity, without grant or with more than one matching grant.
pub(crate) fn granted_write(profiles: &ShareProfiles, contact: &DirectContact) -> Option<bool> {
    let peer = ShareProfiles::contact_remote_identity(contact)?;
    let mut matches = profiles.direct_grants.iter().filter(|grant| {
        peer.device_id == grant.device_id
            && peer.public_key == grant.public_key
            && peer.fingerprint == grant.fingerprint
            && (peer.node_id.is_empty() || peer.node_id == grant.node_id)
    });
    let grant = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(grant.write)
}
