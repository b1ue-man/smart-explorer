use std::io;
use std::sync::{Arc, Mutex};

use super::core::eio;
use super::types::{PeerEndpoint, PeerOpenTarget, PeerPresence, ShareAuthState};

#[derive(Clone)]
pub(super) enum PeerEndpointSource {
    Static(PeerEndpoint),
    Live {
        initial: PeerEndpoint,
        target: PeerOpenTarget,
        auth: Arc<Mutex<ShareAuthState>>,
    },
}

impl PeerEndpointSource {
    pub(super) fn fixed(endpoint: PeerEndpoint) -> Self {
        Self::Static(endpoint)
    }

    pub(super) fn live(
        initial: PeerEndpoint,
        target: PeerOpenTarget,
        auth: Arc<Mutex<ShareAuthState>>,
    ) -> Self {
        Self::Live {
            initial,
            target,
            auth,
        }
    }

    pub(super) fn initial(&self) -> &PeerEndpoint {
        match self {
            Self::Static(endpoint)
            | Self::Live {
                initial: endpoint, ..
            } => endpoint,
        }
    }

    pub(super) fn current(&self) -> io::Result<PeerEndpoint> {
        match self {
            Self::Static(endpoint) => Ok(endpoint.clone()),
            Self::Live {
                initial,
                target,
                auth,
            } => {
                let state = auth
                    .lock()
                    .map_err(|_| eio("Share-State fuer Peer-Routen ist gesperrt"))?;
                let presence = match current_presence(&state, target, initial) {
                    Ok(presence) => presence,
                    Err(error) if error.kind() == io::ErrorKind::NotConnected => {
                        // Presence is routing evidence, not the lifetime of an
                        // already authenticated QUIC session. Keep its pinned
                        // identity usable for the healthy cached connection;
                        // a physical reconnect will still reject these expired
                        // routes in endpoint_addr until fresh Presence arrives.
                        return Ok(initial.clone());
                    }
                    Err(error) => return Err(error),
                };
                validate_identity(initial, &presence)?;
                let mut endpoint = initial.clone();
                endpoint.presence = presence;
                Ok(endpoint)
            }
        }
    }
}

fn current_presence(
    state: &ShareAuthState,
    target: &PeerOpenTarget,
    initial: &PeerEndpoint,
) -> io::Result<PeerPresence> {
    let expected = &initial.presence;
    match target {
        PeerOpenTarget::Direct { contact_id } => {
            let contact = state
                .direct_contacts
                .iter()
                .find(|contact| &contact.id == contact_id)
                .ok_or_else(|| denied("Direktgeraet wurde waehrend der Laufzeit entfernt"))?;
            if contact.access_state != super::types::DirectAccessState::Accepted {
                return Err(denied("Direktgeraet ist nicht mehr freigegeben"));
            }
            if (!contact.expected_fingerprint.is_empty()
                && contact.expected_fingerprint != expected.fingerprint)
                || (!contact.expected_node_id.is_empty()
                    && contact.expected_node_id != expected.node_id)
                || contact
                    .remote_device_id
                    .as_deref()
                    .is_some_and(|device| device != expected.device_id.as_str())
                || contact
                    .remote_public_key
                    .as_deref()
                    .is_some_and(|key| key != expected.public_key.as_str())
                || contact
                    .accepted_public_key
                    .as_deref()
                    .is_some_and(|key| key != expected.public_key.as_str())
            {
                return Err(denied(
                    "Direktgeraet-Pins wurden waehrend des Mounts geaendert",
                ));
            }
            let presence = contact.presence.clone().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "Direktgeraet ist offline")
            })?;
            if (!contact.expected_fingerprint.is_empty()
                && contact.expected_fingerprint != presence.fingerprint)
                || (!contact.expected_node_id.is_empty()
                    && contact.expected_node_id != presence.node_id)
                || contact
                    .remote_device_id
                    .as_deref()
                    .is_some_and(|device| device != presence.device_id.as_str())
                || contact
                    .remote_public_key
                    .as_deref()
                    .is_some_and(|key| key != presence.public_key.as_str())
                || contact
                    .accepted_public_key
                    .as_deref()
                    .is_some_and(|key| key != presence.public_key.as_str())
            {
                return Err(denied("Direktgeraet-Pins stimmen nicht mehr ueberein"));
            }
            Ok(presence)
        }
        PeerOpenTarget::RoomDevice { room_id, device_id } => {
            let room = state
                .rooms
                .iter()
                .find(|room| &room.id == room_id || &room.room_id == room_id)
                .ok_or_else(|| denied("Raum wurde waehrend der Laufzeit entfernt"))?;
            if !room.auto_join {
                return Err(denied("Raum ist nicht mehr aktiv"));
            }
            let member = room
                .members
                .iter()
                .find(|member| &member.device_id == device_id)
                .ok_or_else(|| denied("Raumgeraet wurde waehrend der Laufzeit entfernt"))?;
            if member.blocked {
                return Err(denied("Raumgeraet wurde blockiert"));
            }
            if member.device_id != expected.device_id
                || member.public_key != expected.public_key
                || member.fingerprint != expected.fingerprint
                || (!member.node_id.is_empty() && member.node_id != expected.node_id)
            {
                return Err(denied(
                    "Raumgeraet-Pins wurden waehrend des Mounts geaendert",
                ));
            }
            member.presence.clone().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "Raumgeraet ist offline")
            })
        }
    }
}

fn validate_identity(initial: &PeerEndpoint, current: &PeerPresence) -> io::Result<()> {
    let expected = &initial.presence;
    if current.kind != expected.kind
        || current.relation_id != expected.relation_id
        || current.device_id != expected.device_id
        || current.public_key != expected.public_key
        || current.fingerprint != expected.fingerprint
        || current.node_id != expected.node_id
    {
        return Err(denied(
            "Peer-Identitaet oder Relation hat sich waehrend des Mounts geaendert",
        ));
    }
    if initial
        .expected_node_id
        .as_deref()
        .is_some_and(|node| !node.trim().is_empty() && node != current.node_id.as_str())
    {
        return Err(denied("Iroh NodeId passt nicht zur gepinnten Identitaet"));
    }
    Ok(())
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::{DirectAccessState, DirectContact, ShareStatus};

    #[test]
    fn remote_drive_task_live_endpoint_refreshes_routes_but_not_identity() {
        let initial = endpoint("127.0.0.1:1000", 10);
        let mut contact = contact(initial.presence.clone());
        let auth = Arc::new(Mutex::new(state(contact.clone())));
        let source = PeerEndpointSource::live(
            initial.clone(),
            PeerOpenTarget::Direct {
                contact_id: contact.id.clone(),
            },
            auth.clone(),
        );

        contact.presence.as_mut().unwrap().candidates = vec!["127.0.0.1:2000".into()];
        contact.presence.as_mut().unwrap().expires_at = 20;
        auth.lock().unwrap().direct_contacts = vec![contact];
        let refreshed = source.current().unwrap();
        assert_eq!(refreshed.presence.candidates, ["127.0.0.1:2000"]);
        assert_eq!(refreshed.presence.expires_at, 20);
        assert_eq!(refreshed.relation_secret, initial.relation_secret);

        auth.lock().unwrap().direct_contacts[0].presence = None;
        let offline = source.current().unwrap();
        assert_eq!(offline.presence.candidates, initial.presence.candidates);

        auth.lock().unwrap().direct_contacts[0].expected_node_id = "replacement-node".into();
        assert_eq!(
            source.current().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        auth.lock().unwrap().direct_contacts[0].expected_node_id = "node".into();
        auth.lock().unwrap().direct_contacts[0].presence = Some(initial.presence.clone());
        auth.lock().unwrap().direct_contacts[0]
            .presence
            .as_mut()
            .unwrap()
            .node_id = "replacement-node".into();
        assert_eq!(
            source.current().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    fn endpoint(candidate: &str, expires_at: i64) -> PeerEndpoint {
        PeerEndpoint {
            label: "Peer".into(),
            scope: super::super::types::ShareScope::Direct {
                contact_id: "contact".into(),
            },
            presence: PeerPresence {
                kind: "direct".into(),
                relation_id: "lookup".into(),
                device_id: "remote".into(),
                device_name: "Remote".into(),
                public_key: "public".into(),
                fingerprint: "fingerprint".into(),
                node_id: "node".into(),
                relay_url: String::new(),
                candidates: vec![candidate.into()],
                expires_at,
                nonce: "nonce".into(),
                proof: "proof".into(),
            },
            relation_secret: vec![7; 32],
            expected_node_id: Some("node".into()),
        }
    }

    fn contact(presence: PeerPresence) -> DirectContact {
        DirectContact {
            id: "contact".into(),
            display_name: "Remote".into(),
            lookup_id: "lookup".into(),
            expected_fingerprint: "fingerprint".into(),
            expected_node_id: "node".into(),
            remote_device_id: Some("remote".into()),
            remote_public_key: Some("public".into()),
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Available,
            last_error: None,
            presence: Some(presence),
            access_state: DirectAccessState::Accepted,
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: Some("public".into()),
        }
    }

    fn state(contact: DirectContact) -> ShareAuthState {
        let secret = iroh::SecretKey::from_bytes(&[19; 32]);
        let node_id = secret.public().to_string();
        ShareAuthState {
            identity: crate::share::ShareIdentity {
                device_id: "local".into(),
                device_name: "Local".into(),
                direct_lookup_id: "local-lookup".into(),
                public_key: node_id.clone(),
                fingerprint: crate::share::core::public_fingerprint(node_id.as_bytes()),
                node_id,
                iroh_secret: secret,
                direct_secret: [0; 32],
            },
            direct_secret: vec![0; 32],
            default_direct_exports: Default::default(),
            direct_contacts: vec![contact],
            direct_grants: Vec::new(),
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 0,
        }
    }
}
