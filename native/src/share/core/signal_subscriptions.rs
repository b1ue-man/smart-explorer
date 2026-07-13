use std::collections::{BTreeSet, HashSet};

use super::types::{DirectContact, RoomProfile, ShareAuthState};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SubscriptionTeardownPlan {
    pub(super) direct_lookup_ids: Vec<String>,
    pub(super) room_ids: Vec<String>,
}

pub(super) fn plan_subscription_teardown(
    current: &ShareAuthState,
    next_direct: &[DirectContact],
    next_rooms: &[RoomProfile],
) -> SubscriptionTeardownPlan {
    let retained_direct = next_direct
        .iter()
        .filter(|contact| contact.auto_connect)
        .map(|contact| contact.lookup_id.as_str())
        .collect::<HashSet<_>>();
    let retained_rooms = next_rooms
        .iter()
        .filter(|room| room.auto_join)
        .map(|room| room.room_id.as_str())
        .collect::<HashSet<_>>();

    SubscriptionTeardownPlan {
        direct_lookup_ids: current
            .direct_contacts
            .iter()
            .filter(|contact| {
                contact.auto_connect && !retained_direct.contains(contact.lookup_id.as_str())
            })
            .map(|contact| contact.lookup_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        room_ids: current
            .rooms
            .iter()
            .filter(|room| room.auto_join && !retained_rooms.contains(room.room_id.as_str()))
            .map(|room| room.room_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::core::public_fingerprint;
    use crate::share::fs::ShareExportConfig;
    use crate::share::identity::ShareIdentity;
    use crate::share::types::{DirectAccessState, ShareStatus};

    #[test]
    fn removed_active_subscriptions_are_torn_down() {
        let current = state(
            vec![contact("lookup-b", true), contact("lookup-a", true)],
            vec![room("room-b", true), room("room-a", true)],
        );

        let plan = plan_subscription_teardown(&current, &[], &[]);

        assert_eq!(plan.direct_lookup_ids, ["lookup-a", "lookup-b"]);
        assert_eq!(plan.room_ids, ["room-a", "room-b"]);
    }

    #[test]
    fn disabling_active_subscriptions_tears_them_down() {
        let current = state(vec![contact("lookup-a", true)], vec![room("room-a", true)]);

        let plan = plan_subscription_teardown(
            &current,
            &[contact("lookup-a", false)],
            &[room("room-a", false)],
        );

        assert_eq!(plan.direct_lookup_ids, ["lookup-a"]);
        assert_eq!(plan.room_ids, ["room-a"]);
    }

    #[test]
    fn unchanged_active_subscriptions_are_retained() {
        let current = state(
            vec![contact("lookup-a", true), contact("lookup-off", false)],
            vec![room("room-a", true), room("room-off", false)],
        );

        let plan = plan_subscription_teardown(
            &current,
            &[contact("lookup-a", true)],
            &[room("room-a", true)],
        );

        assert_eq!(plan, SubscriptionTeardownPlan::default());
    }

    #[test]
    fn duplicate_relation_ids_produce_one_teardown_each() {
        let current = state(
            vec![contact("lookup-a", true), contact("lookup-a", true)],
            vec![room("room-a", true), room("room-a", true)],
        );

        let plan = plan_subscription_teardown(
            &current,
            &[contact("lookup-a", false), contact("lookup-a", false)],
            &[room("room-a", false), room("room-a", false)],
        );

        assert_eq!(plan.direct_lookup_ids, ["lookup-a"]);
        assert_eq!(plan.room_ids, ["room-a"]);
    }

    fn state(direct_contacts: Vec<DirectContact>, rooms: Vec<RoomProfile>) -> ShareAuthState {
        let iroh_secret = iroh::SecretKey::from_bytes(&[7; 32]);
        let public_key = iroh_secret.public().to_string();
        ShareAuthState {
            identity: ShareIdentity {
                device_id: "device-local".into(),
                device_name: "Local".into(),
                direct_lookup_id: "lookup-local".into(),
                fingerprint: public_fingerprint(public_key.as_bytes()),
                node_id: public_key.clone(),
                public_key,
                iroh_secret,
                direct_secret: [8; 32],
            },
            direct_secret: vec![8; 32],
            default_direct_exports: ShareExportConfig::default(),
            direct_contacts,
            direct_grants: Vec::new(),
            rooms,
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: HashSet::new(),
            direct_online: true,
            authorization_epoch: 0,
        }
    }

    fn contact(lookup_id: &str, auto_connect: bool) -> DirectContact {
        DirectContact {
            id: format!("contact-{lookup_id}"),
            display_name: lookup_id.into(),
            lookup_id: lookup_id.into(),
            expected_fingerprint: "00".repeat(16),
            expected_node_id: format!("node-{lookup_id}"),
            remote_device_id: None,
            remote_public_key: None,
            auto_connect,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Waiting,
            last_error: None,
            presence: None,
            access_state: DirectAccessState::Pending,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
        }
    }

    fn room(room_id: &str, auto_join: bool) -> RoomProfile {
        RoomProfile {
            id: format!("profile-{room_id}"),
            name: room_id.into(),
            room_id: room_id.into(),
            auto_join,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: Vec::new(),
            exports: ShareExportConfig::default(),
        }
    }
}
