//! Which stored relation reaches a confirmed device (design 2.2): the first
//! present one, a Direct contact before a room membership („direkt zuerst“,
//! `docs/ARCHITEKTUR.md`). Among several of one kind the most recent
//! presence wins, then the smaller relation ID. RDP, the device feed,
//! programs and TCP all use this one choice.
use crate::share::core::now_secs;
use crate::share::profiles::ShareProfiles;
use crate::share::types::{PeerOpenTarget, PeerPresence};

/// A candidate relation: kind rank (0 Direct, 1 room), presence expiry
/// (later = more recent announcement), relation ID, target.
type Candidate = (u8, i64, String, PeerOpenTarget);

fn present(
    presence: Option<&PeerPresence>,
    device_id: &str,
    public_key: &str,
    now: i64,
) -> Option<i64> {
    let presence = presence?;
    (presence.is_current_at(now)
        && presence.device_id == device_id
        && presence.public_key == public_key)
        .then_some(presence.expires_at)
}

/// The relation to use for `device_id`/`public_key` at `now`; `None` when no
/// stored relation to that exact identity is present.
pub(crate) fn present_relation_at(
    profiles: &ShareProfiles,
    device_id: &str,
    public_key: &str,
    now: i64,
) -> Option<PeerOpenTarget> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for contact in &profiles.direct_contacts {
        if contact.remote_device_id.as_deref() != Some(device_id)
            || contact.remote_public_key.as_deref() != Some(public_key)
        {
            continue;
        }
        if let Some(seen) = present(contact.presence.as_ref(), device_id, public_key, now) {
            candidates.push((
                0,
                seen,
                contact.id.clone(),
                PeerOpenTarget::Direct {
                    contact_id: contact.id.clone(),
                },
            ));
        }
    }
    for room in profiles.rooms.iter().filter(|room| room.auto_join) {
        for member in &room.members {
            if member.device_id != device_id || member.public_key != public_key || member.blocked {
                continue;
            }
            if let Some(seen) = present(member.presence.as_ref(), device_id, public_key, now) {
                candidates.push((
                    1,
                    seen,
                    room.room_id.clone(),
                    PeerOpenTarget::RoomDevice {
                        room_id: room.room_id.clone(),
                        device_id: device_id.into(),
                    },
                ));
            }
        }
    }
    candidates
        .into_iter()
        .min_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.cmp(&b.2))
        })
        .map(|candidate| candidate.3)
}

/// Design 9.1 contract: the present relation at the host clock's now.
// Consumers: RDP open, device feed, programs, TCP (work package WP2).
#[allow(dead_code)]
pub(crate) fn present_relation(
    profiles: &ShareProfiles,
    device_id: &str,
    public_key: &str,
) -> Option<PeerOpenTarget> {
    present_relation_at(profiles, device_id, public_key, now_secs())
}
