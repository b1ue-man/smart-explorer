use std::io;
use std::sync::{Arc, Mutex};

use super::core::{eio, hmac_proof, random_token, verify_hmac};
use super::exec_policy::ExecGrant;
use super::exec_protocol::{ExecClientHello, ExecServerHello};
use super::exec_types::{ExecAuthorization, ExecPrincipal, EXEC_CAPABILITY, EXEC_PROTOCOL_VERSION};
use super::identity::ShareIdentity;
use super::profiles::{fingerprint_matches, ShareProfiles};
use super::session::relation_kind_id;
use super::types::{DirectGrantState, PeerEndpoint, ShareAuthState};

pub(crate) struct AuthorizedExecPeer {
    pub(crate) principal: ExecPrincipal,
    pub(crate) authorization: ExecAuthorization,
    /// The grant as authorized: run-as account and program list apply to
    /// this connection's start.
    pub(crate) grant: ExecGrant,
}

/// Why an Exec handshake was refused. Every reason answers the peer with the
/// same `permission_denied`; only `Authentication` counts as a failed
/// authentication for the host's penalties (`auth_penalty.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecRejection {
    /// Protocol fields, pinned identity or relation proof did not verify.
    Authentication,
    /// A verified device without Exec permission, or an offline relation.
    NotAuthorized,
}

impl ExecRejection {
    /// Only an identity or proof that did not verify is a failed
    /// authentication. A verified device whose Exec is off, and an offline
    /// relation, are not: they must never lock a paired device out.
    pub(crate) fn counts_as_failed_authentication(self) -> bool {
        matches!(self, Self::Authentication)
    }

    pub(crate) fn audit_reason(self) -> &'static str {
        match self {
            Self::Authentication => "authentication failed",
            Self::NotAuthorized => "exec not permitted",
        }
    }
}

impl From<ExecRejection> for io::Error {
    fn from(_: ExecRejection) -> Self {
        denied()
    }
}

pub(crate) fn build_client_hello(
    server: &ExecServerHello,
    endpoint: &PeerEndpoint,
    identity: &ShareIdentity,
) -> io::Result<ExecClientHello> {
    validate_server(server, endpoint)?;
    let (relation_kind, relation_id) = relation_kind_id(endpoint);
    let client_nonce = random_token(18).map_err(eio)?;
    let mut hello = ExecClientHello {
        protocol_version: EXEC_PROTOCOL_VERSION,
        capability: EXEC_CAPABILITY.into(),
        relation_kind: relation_kind.into(),
        relation_id,
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        client_nonce,
        proof: String::new(),
    };
    hello.proof = hmac_proof(&endpoint.relation_secret, &transcript(server, &hello));
    Ok(hello)
}

pub(crate) fn authorize_client_hello(
    server: &ExecServerHello,
    hello: &ExecClientHello,
    remote_node: &str,
    auth: &Arc<Mutex<ShareAuthState>>,
) -> Result<AuthorizedExecPeer, ExecRejection> {
    let state = auth.lock().map_err(|_| ExecRejection::NotAuthorized)?;
    authorize_client_hello_in(server, hello, remote_node, &state)
}

/// Order: protocol and pinned identity, then the relation proof (HMAC over
/// the transcript), then the Exec permission of the proven device.
fn authorize_client_hello_in(
    server: &ExecServerHello,
    hello: &ExecClientHello,
    remote_node: &str,
    state: &ShareAuthState,
) -> Result<AuthorizedExecPeer, ExecRejection> {
    validate_common(server, hello, remote_node, state)?;
    let grant = match hello.relation_kind.as_str() {
        "direct" if hello.relation_id == state.identity.direct_lookup_id => {
            if !state.direct_online {
                return Err(ExecRejection::NotAuthorized);
            }
            let grant = state
                .direct_grants
                .iter()
                .find(|grant| {
                    grant.device_id == hello.device_id
                        && grant.public_key == hello.public_key
                        && grant.fingerprint == hello.fingerprint
                        && grant.node_id == hello.node_id
                        && grant.state == DirectGrantState::Accepted
                })
                .ok_or(ExecRejection::Authentication)?;
            if !verify_hmac(
                state.direct_secret.as_slice(),
                &transcript(server, hello),
                &hello.proof,
            ) {
                return Err(ExecRejection::Authentication);
            }
            if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
                return Err(ExecRejection::NotAuthorized);
            }
            &grant.exec
        }
        "room" => {
            let room = state
                .rooms
                .iter()
                .find(|room| room.room_id == hello.relation_id && room.auto_join)
                .ok_or(ExecRejection::Authentication)?;
            let member = room
                .members
                .iter()
                .find(|member| {
                    member.device_id == hello.device_id
                        && member.public_key == hello.public_key
                        && member.fingerprint == hello.fingerprint
                        && member.node_id == hello.node_id
                        && member.is_admitted()
                })
                .ok_or(ExecRejection::Authentication)?;
            let secret = ShareProfiles::room_secret_checked(room)
                .map_err(|_| ExecRejection::NotAuthorized)?
                .ok_or(ExecRejection::NotAuthorized)?;
            if !verify_hmac(&secret, &transcript(server, hello), &hello.proof) {
                return Err(ExecRejection::Authentication);
            }
            if !fingerprint_matches(&member.public_key, &member.fingerprint) {
                return Err(ExecRejection::NotAuthorized);
            }
            &member.exec
        }
        _ => return Err(ExecRejection::Authentication),
    };
    if !grant.enabled {
        return Err(ExecRejection::NotAuthorized);
    }
    authorized(state, hello, grant)
}

fn authorized(
    state: &ShareAuthState,
    hello: &ExecClientHello,
    grant: &ExecGrant,
) -> Result<AuthorizedExecPeer, ExecRejection> {
    Ok(AuthorizedExecPeer {
        principal: ExecPrincipal {
            relation_kind: hello.relation_kind.clone(),
            relation_id: hello.relation_id.clone(),
            device_id: hello.device_id.clone(),
            device_name: hello.device_name.clone(),
            public_key: hello.public_key.clone(),
            fingerprint: hello.fingerprint.clone(),
            node_id: hello.node_id.clone(),
        },
        authorization: ExecAuthorization {
            policy_revision: grant.policy_revision,
            authorization_epoch: state.authorization_epoch,
            session_id: random_token(18).map_err(|_| ExecRejection::NotAuthorized)?,
        },
        grant: grant.clone(),
    })
}

fn validate_server(server: &ExecServerHello, endpoint: &PeerEndpoint) -> io::Result<()> {
    if server.protocol_version != EXEC_PROTOCOL_VERSION
        || server.capability != EXEC_CAPABILITY
        || server.server_device_id != endpoint.presence.device_id
        || server.server_public_key != endpoint.presence.public_key
        || server.server_fingerprint != endpoint.presence.fingerprint
        || server.server_node_id != endpoint.presence.node_id
        || !fingerprint_matches(&server.server_public_key, &server.server_fingerprint)
    {
        return Err(denied());
    }
    if endpoint
        .expected_node_id
        .as_ref()
        .is_some_and(|expected| !expected.is_empty() && expected != &server.server_node_id)
    {
        return Err(denied());
    }
    Ok(())
}

fn validate_common(
    server: &ExecServerHello,
    hello: &ExecClientHello,
    remote_node: &str,
    state: &ShareAuthState,
) -> Result<(), ExecRejection> {
    if server.protocol_version != EXEC_PROTOCOL_VERSION
        || server.capability != EXEC_CAPABILITY
        || hello.protocol_version != EXEC_PROTOCOL_VERSION
        || hello.capability != EXEC_CAPABILITY
        || hello.node_id != remote_node
        || server.server_device_id != state.identity.device_id
        || server.server_public_key != state.identity.public_key
        || server.server_fingerprint != state.identity.fingerprint
        || server.server_node_id != state.identity.node_id
        || server.challenge.len() < 24
        || hello.client_nonce.len() < 24
        || !fingerprint_matches(&hello.public_key, &hello.fingerprint)
    {
        return Err(ExecRejection::Authentication);
    }
    Ok(())
}

pub(crate) fn transcript(server: &ExecServerHello, client: &ExecClientHello) -> String {
    format!(
        "smart-explorer/share-exec/v2|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|client-to-server|{}",
        server.challenge,
        client.client_nonce,
        client.relation_kind,
        client.relation_id,
        client.device_id,
        client.public_key,
        client.fingerprint,
        client.node_id,
        server.server_device_id,
        server.server_public_key,
        server.server_fingerprint,
        server.server_node_id,
        EXEC_CAPABILITY,
    )
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "exec authentication failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::core::public_fingerprint;
    use crate::share::exec_policy::ExecGrant;
    use crate::share::types::DirectGrant;

    #[test]
    fn proof_is_bound_to_the_fresh_server_challenge() {
        let identity = identity("server-a", "Server");
        let server = server_hello(&identity, "challenge-one-0123456789");
        let mut client = client(&identity.direct_lookup_id);
        client.proof = hmac_proof(b"secret", &transcript(&server, &client));
        assert!(verify_hmac(
            b"secret",
            &transcript(&server, &client),
            &client.proof
        ));
        assert!(!verify_hmac(
            b"secret",
            &transcript(
                &server_hello(&identity, "challenge-two-0123456789"),
                &client,
            ),
            &client.proof
        ));
    }

    #[test]
    fn exact_identity_and_separate_exec_permission_are_required() {
        let identity = identity("server-b", "Server");
        let server = server_hello(&identity, "challenge-one-0123456789");
        let mut client = client(&identity.direct_lookup_id);
        let mut state = ShareAuthState {
            direct_secret: b"secret".to_vec(),
            identity,
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: vec![DirectGrant {
                device_id: client.device_id.clone(),
                device_name: client.device_name.clone(),
                public_key: client.public_key.clone(),
                fingerprint: client.fingerprint.clone(),
                node_id: client.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: 1,
                exec: ExecGrant::default(),
                write: false,
            }],
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 7,
        };
        client.proof = hmac_proof(b"secret", &transcript(&server, &client));
        assert_eq!(
            authorize_client_hello_in(&server, &client, &client.node_id, &state).err(),
            Some(ExecRejection::NotAuthorized)
        );
        state.direct_grants[0].exec.enabled = true;
        state.direct_grants[0].exec.policy_revision = 3;
        let authorized =
            authorize_client_hello_in(&server, &client, &client.node_id, &state).unwrap();
        assert_eq!(authorized.authorization.policy_revision, 3);
        assert_eq!(authorized.grant, state.direct_grants[0].exec);
        let mut forged = client.clone();
        forged.proof = hmac_proof(b"wrong", &transcript(&server, &forged));
        assert_eq!(
            authorize_client_hello_in(&server, &forged, &forged.node_id, &state).err(),
            Some(ExecRejection::Authentication)
        );
        state.direct_grants[0].node_id = "changed".into();
        assert_eq!(
            authorize_client_hello_in(&server, &client, &client.node_id, &state).err(),
            Some(ExecRejection::Authentication)
        );
    }

    #[test]
    fn luecken_task_proof_is_checked_before_the_exec_permission() {
        let identity = identity("server-c", "Server");
        let server = server_hello(&identity, "challenge-one-0123456789");
        let mut client = client(&identity.direct_lookup_id);
        let state = ShareAuthState {
            direct_secret: b"secret".to_vec(),
            identity,
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: vec![DirectGrant {
                device_id: client.device_id.clone(),
                device_name: client.device_name.clone(),
                public_key: client.public_key.clone(),
                fingerprint: client.fingerprint.clone(),
                node_id: client.node_id.clone(),
                state: DirectGrantState::Accepted,
                updated_at: 1,
                exec: ExecGrant::default(),
                write: false,
            }],
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 1,
        };
        // Exec is off: a wrong proof is still an authentication failure, so
        // probing never learns whether a grant exists.
        client.proof = hmac_proof(b"wrong", &transcript(&server, &client));
        assert_eq!(
            authorize_client_hello_in(&server, &client, &client.node_id, &state).err(),
            Some(ExecRejection::Authentication)
        );
        client.proof = hmac_proof(b"secret", &transcript(&server, &client));
        let off = authorize_client_hello_in(&server, &client, &client.node_id, &state).err();
        assert_eq!(off, Some(ExecRejection::NotAuthorized));
        // Identity and proof failures count against the penalty account;
        // a verified device with Exec off never does.
        assert!(ExecRejection::Authentication.counts_as_failed_authentication());
        assert!(!off.unwrap().counts_as_failed_authentication());
    }

    fn identity(device_id: &str, device_name: &str) -> ShareIdentity {
        let mut bytes = [17u8; 32];
        for (index, byte) in device_id.bytes().enumerate() {
            let slot = index % bytes.len();
            bytes[slot] = bytes[slot].wrapping_mul(31).wrapping_add(byte);
        }
        let secret = iroh::SecretKey::from_bytes(&bytes);
        let node_id = secret.public().to_string();
        ShareIdentity {
            device_id: device_id.into(),
            device_name: device_name.into(),
            direct_lookup_id: format!("lookup-{device_id}"),
            public_key: node_id.clone(),
            fingerprint: public_fingerprint(node_id.as_bytes()),
            node_id,
            iroh_secret: secret,
            direct_secret: [0; 32],
        }
    }

    fn server_hello(identity: &ShareIdentity, challenge: &str) -> ExecServerHello {
        ExecServerHello::new(
            challenge.into(),
            identity.device_id.clone(),
            identity.public_key.clone(),
            identity.fingerprint.clone(),
            identity.node_id.clone(),
        )
    }

    fn client(relation_id: &str) -> ExecClientHello {
        let identity = identity("client", "Client");
        ExecClientHello {
            protocol_version: EXEC_PROTOCOL_VERSION,
            capability: EXEC_CAPABILITY.into(),
            relation_kind: "direct".into(),
            relation_id: relation_id.into(),
            device_id: identity.device_id,
            device_name: identity.device_name,
            public_key: identity.public_key,
            fingerprint: identity.fingerprint,
            node_id: identity.node_id,
            client_nonce: "client-nonce-012345678901".into(),
            proof: String::new(),
        }
    }
}
