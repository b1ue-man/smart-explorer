use std::io;
use std::sync::{Arc, Mutex};

use super::access_authorize::{exec_connection_allowed, principal_grant};
use super::core::{eio, hmac_proof, now_secs, random_token};
use super::exec_access_view::client_features;
use super::exec_policy::ExecGrant;
use super::exec_protocol::{ExecClientHello, ExecServerHello};
use super::exec_types::{ExecAuthorization, ExecPrincipal, EXEC_CAPABILITY, EXEC_PROTOCOL_VERSION};
use super::identity::ShareIdentity;
use super::profiles::fingerprint_matches;
use super::relation_proof::{
    prove_relation, relation_transcript, ProvenPrincipal, RelationClientHello, RelationRejection,
    RelationServerHello, EXEC_TRANSCRIPT_PREFIX,
};
use super::session::relation_kind_id;
use super::types::{PeerEndpoint, ShareAuthState};

pub(crate) struct AuthorizedExecPeer {
    pub(crate) principal: ExecPrincipal,
    pub(crate) authorization: ExecAuthorization,
    /// The grant at the handshake. The start is checked against the grant
    /// of that moment (`exec_admission.rs`).
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

impl From<RelationRejection> for ExecRejection {
    fn from(rejection: RelationRejection) -> Self {
        match rejection {
            RelationRejection::Authentication => Self::Authentication,
            RelationRejection::NotAuthorized => Self::NotAuthorized,
        }
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
        client_features: client_features(),
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
    authorize_client_hello_in(server, hello, remote_node, &state, now_secs())
}

/// Order: protocol and pinned identity, then the relation proof (HMAC over
/// the transcript), then the Exec permission of the proven device at `now`:
/// commands or terminal not off and the grant not ended.
pub(super) fn authorize_client_hello_in(
    server: &ExecServerHello,
    hello: &ExecClientHello,
    remote_node: &str,
    state: &ShareAuthState,
    now: i64,
) -> Result<AuthorizedExecPeer, ExecRejection> {
    if server.protocol_version != EXEC_PROTOCOL_VERSION || server.capability != EXEC_CAPABILITY {
        return Err(ExecRejection::Authentication);
    }
    let proven = prove_relation(
        &relation_server(server),
        &relation_client(hello),
        remote_node,
        state,
    )?;
    let grant = principal_grant(state, &proven).ok_or(ExecRejection::NotAuthorized)?;
    if !exec_connection_allowed(grant, now) {
        return Err(ExecRejection::NotAuthorized);
    }
    authorized(state, &proven, grant)
}

fn authorized(
    state: &ShareAuthState,
    proven: &ProvenPrincipal,
    grant: &ExecGrant,
) -> Result<AuthorizedExecPeer, ExecRejection> {
    Ok(AuthorizedExecPeer {
        principal: ExecPrincipal::from(proven),
        authorization: ExecAuthorization {
            policy_revision: grant.policy_revision,
            authorization_epoch: state.authorization_epoch,
            session_id: random_token(18).map_err(|_| ExecRejection::NotAuthorized)?,
        },
        grant: grant.clone(),
    })
}

/// The Exec server hello as a relation handshake.
pub(super) fn relation_server(server: &ExecServerHello) -> RelationServerHello {
    RelationServerHello {
        transcript_prefix: EXEC_TRANSCRIPT_PREFIX.into(),
        protocol_version: server.protocol_version,
        capability: server.capability.clone(),
        challenge: server.challenge.clone(),
        server_device_id: server.server_device_id.clone(),
        server_public_key: server.server_public_key.clone(),
        server_fingerprint: server.server_fingerprint.clone(),
        server_node_id: server.server_node_id.clone(),
    }
}

/// The Exec client hello as a relation handshake; `client_features` are not
/// part of the proof.
pub(super) fn relation_client(hello: &ExecClientHello) -> RelationClientHello {
    RelationClientHello {
        protocol_version: hello.protocol_version,
        capability: hello.capability.clone(),
        relation_kind: hello.relation_kind.clone(),
        relation_id: hello.relation_id.clone(),
        device_id: hello.device_id.clone(),
        device_name: hello.device_name.clone(),
        public_key: hello.public_key.clone(),
        fingerprint: hello.fingerprint.clone(),
        node_id: hello.node_id.clone(),
        client_nonce: hello.client_nonce.clone(),
        proof: hello.proof.clone(),
    }
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

/// The text the client's proof is an HMAC over; the client checked the
/// server's protocol version and capability before (`validate_server`).
pub(crate) fn transcript(server: &ExecServerHello, client: &ExecClientHello) -> String {
    relation_transcript(&relation_server(server), &relation_client(client))
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
    use crate::share::core::{public_fingerprint, verify_hmac};
    use crate::share::exec_policy::ExecGrant;
    use crate::share::types::{DirectGrant, DirectGrantState};

    /// Unix seconds of the checks; the grants of these tests have no end.
    const NOW: i64 = 1_760_000_000;

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
            authorize_client_hello_in(&server, &client, &client.node_id, &state, NOW).err(),
            Some(ExecRejection::NotAuthorized)
        );
        state.direct_grants[0].exec.enabled = true;
        state.direct_grants[0].exec.policy_revision = 3;
        let authorized =
            authorize_client_hello_in(&server, &client, &client.node_id, &state, NOW).unwrap();
        assert_eq!(authorized.authorization.policy_revision, 3);
        assert_eq!(authorized.grant, state.direct_grants[0].exec);
        let mut forged = client.clone();
        forged.proof = hmac_proof(b"wrong", &transcript(&server, &forged));
        assert_eq!(
            authorize_client_hello_in(&server, &forged, &forged.node_id, &state, NOW).err(),
            Some(ExecRejection::Authentication)
        );
        state.direct_grants[0].node_id = "changed".into();
        assert_eq!(
            authorize_client_hello_in(&server, &client, &client.node_id, &state, NOW).err(),
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
            authorize_client_hello_in(&server, &client, &client.node_id, &state, NOW).err(),
            Some(ExecRejection::Authentication)
        );
        client.proof = hmac_proof(b"secret", &transcript(&server, &client));
        let off = authorize_client_hello_in(&server, &client, &client.node_id, &state, NOW).err();
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
            client_features: Vec::new(),
        }
    }
}
