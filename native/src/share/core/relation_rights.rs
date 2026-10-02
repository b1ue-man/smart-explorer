//! Rights of a relation as seen by the host (contract V5).
//!
//! [`SessionAuthorization`] is what one authorized filesystem stream may do;
//! [`RestrictionSet`] names exactly whose rights a configuration change
//! narrowed („Recht eingeschränkt für (Schlüssel, Beziehung)“), so only those
//! sessions, leases, transfers, Exec jobs and repairs end (FA3).
use super::fs::ShareExportConfig;
use super::types::{DirectContact, DirectGrant, RoomProfile, ShareAuthState, ShareStatus};

/// Result of authorizing one incoming filesystem stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionAuthorization {
    /// Exports this relation sees (Direct: `default_direct_exports`, room:
    /// `room.exports`).
    pub(crate) exports: ShareExportConfig,
    /// Write right of the relation (Direct `DirectGrant.write`, room
    /// `RoomPolicy.members_may_write`). A writing request is allowed only for
    /// `may_write && root.access == ReadWrite`.
    pub(crate) may_write: bool,
}

impl SessionAuthorization {
    pub(crate) fn direct(exports: ShareExportConfig, grant: &DirectGrant) -> Self {
        Self {
            exports,
            may_write: grant.write,
        }
    }

    pub(crate) fn room(room: &RoomProfile) -> Self {
        Self {
            exports: room.exports.clone(),
            may_write: room.policy.members_may_write,
        }
    }
}

/// Relation in which a right was restricted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RelationScope {
    /// The Direct relation with one peer in both directions (incoming over
    /// this device's lookup id, outgoing over the contact's lookup id).
    Direct,
    /// One room (`RoomProfile.room_id`).
    Room { room_id: String },
}

/// Identity a restriction applies to. A session, lease, transfer, Exec job or
/// repair matches when its peer key **or** node id is equal; empty values
/// never match.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PrincipalKey {
    pub(crate) public_key: String,
    pub(crate) node_id: String,
}

impl PrincipalKey {
    pub(crate) fn matches(&self, public_key: &str, node_id: &str) -> bool {
        (!self.public_key.is_empty() && self.public_key == public_key)
            || (!self.node_id.is_empty() && self.node_id == node_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RestrictionReason {
    /// Contact or grant removed („Entfernen“).
    Removed,
    /// Grant ignored or member blocked („Sperren“).
    Blocked,
    /// Grant suspended for re-confirmation (code rotation, identity repair).
    Reconfirm,
    /// Peer key or node changed, or a pin was replaced.
    IdentityChanged,
    /// Write right withdrawn (grant, room, or export access).
    WriteRevoked,
    /// Exec grant withdrawn.
    ExecRevoked,
    /// Exports removed or narrowed.
    ExportsNarrowed,
    /// Relation inactive: room left or deactivated, Direct offline, contact
    /// access withdrawn.
    RelationInactive,
    /// Not attributable to a key and relation: close everything (pre-V5).
    Unattributed,
}

/// Event „Recht eingeschränkt für (Schlüssel, Beziehung)“.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RightsRestriction {
    pub(crate) relation: RelationScope,
    /// `None`: every principal of the relation.
    pub(crate) principal: Option<PrincipalKey>,
    pub(crate) reason: RestrictionReason,
}

impl RightsRestriction {
    pub(crate) fn affects(
        &self,
        relation_kind: &str,
        relation_id: &str,
        public_key: &str,
        node_id: &str,
    ) -> bool {
        let relation = match &self.relation {
            RelationScope::Direct => relation_kind == "direct",
            RelationScope::Room { room_id } => relation_kind == "room" && room_id == relation_id,
        };
        relation
            && self
                .principal
                .as_ref()
                .is_none_or(|principal| principal.matches(public_key, node_id))
    }
}

/// All restrictions of one configuration change. Empty: nothing may be
/// closed (presence and other runtime data, new members, any extension);
/// everything: close all as before V5.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestrictionSet {
    everything: Option<RestrictionReason>,
    items: Vec<RightsRestriction>,
}

impl RestrictionSet {
    pub(crate) fn everything(reason: RestrictionReason) -> Self {
        Self {
            everything: Some(reason),
            items: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, restriction: RightsRestriction) {
        if !self.items.contains(&restriction) {
            self.items.push(restriction);
        }
    }

    pub(crate) fn merge(&mut self, other: RestrictionSet) {
        if self.everything.is_none() {
            self.everything = other.everything;
        }
        for restriction in other.items {
            self.push(restriction);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.everything.is_none() && self.items.is_empty()
    }

    /// `Some(reason)` when the change cannot be scoped and everything closes.
    pub(crate) fn everything_reason(&self) -> Option<RestrictionReason> {
        self.everything
    }

    pub(crate) fn items(&self) -> &[RightsRestriction] {
        &self.items
    }

    /// Whether a session, lease, transfer, Exec job or repair of this
    /// principal is hit. `relation_kind` is `"direct"` or `"room"` as in
    /// `PeerHello.relation_kind`; `relation_id` is the room id (ignored for
    /// Direct).
    pub(crate) fn affects(
        &self,
        relation_kind: &str,
        relation_id: &str,
        public_key: &str,
        node_id: &str,
    ) -> bool {
        self.everything.is_some()
            || self
                .items
                .iter()
                .any(|item| item.affects(relation_kind, relation_id, public_key, node_id))
    }
}

/// Restrictions between the running authorization state and a candidate that
/// replaces it (relations and exports). Runtime fields (presence, status,
/// `last_seen`, LAN routes, member routes and names) never restrict anything;
/// additions are extensions.
///
/// Contract stub: any other difference restricts everything, as before V5.
pub(crate) fn authorization_restrictions(
    current: &ShareAuthState,
    candidate: &ShareAuthState,
) -> RestrictionSet {
    if AuthorizationView::of(current) == AuthorizationView::of(candidate) {
        RestrictionSet::default()
    } else {
        RestrictionSet::everything(RestrictionReason::Unattributed)
    }
}

/// Everything of a state that grants or narrows a right, without runtime data.
#[derive(PartialEq)]
struct AuthorizationView<'a> {
    device_id: &'a str,
    lookup_id: &'a str,
    public_key: &'a str,
    node_id: &'a str,
    direct_online: bool,
    exports: &'a ShareExportConfig,
    grants: &'a [DirectGrant],
    contacts: Vec<DirectContact>,
    rooms: Vec<RoomProfile>,
}

impl<'a> AuthorizationView<'a> {
    fn of(state: &'a ShareAuthState) -> Self {
        Self {
            device_id: &state.identity.device_id,
            lookup_id: &state.identity.direct_lookup_id,
            public_key: &state.identity.public_key,
            node_id: &state.identity.node_id,
            direct_online: state.direct_online,
            exports: &state.default_direct_exports,
            grants: &state.direct_grants,
            contacts: state
                .direct_contacts
                .iter()
                .map(without_contact_runtime)
                .collect(),
            rooms: state.rooms.iter().map(without_room_runtime).collect(),
        }
    }
}

fn without_contact_runtime(contact: &DirectContact) -> DirectContact {
    let mut contact = contact.clone();
    contact.status = ShareStatus::default();
    contact.last_seen = None;
    contact.last_error = None;
    contact.presence = None;
    contact.lan_candidates.clear();
    contact.lan_seen_at = None;
    contact.lan_uplink = None;
    contact
}

fn without_room_runtime(room: &RoomProfile) -> RoomProfile {
    let mut room = room.clone();
    room.status = ShareStatus::default();
    room.last_seen = None;
    for member in &mut room.members {
        member.device_name.clear();
        member.relay_url.clear();
        member.candidates.clear();
        member.last_seen = None;
        member.status = ShareStatus::default();
        member.presence = None;
    }
    room
}
