//! Proof of a relation for a protocol handshake (contract V7, E point 2):
//! the pinned identity of the connecting device, the HMAC over a transcript
//! that starts with the protocol's own prefix, and the active relation
//! (Direct or room) it belongs to. It decides nothing about capabilities:
//! Exec (`exec_auth.rs`) and the screen protocol of S4 ask `capability_for`
//! afterwards. The checks and their order are those of the Exec handshake.
use crate::share::core::verify_hmac;
use crate::share::exec_types::ExecPrincipal;
use crate::share::identity::code_renewal_registry::retiring_secret;
use crate::share::profiles::{fingerprint_matches, ShareProfiles};
use crate::share::types::{DirectGrantState, ShareAuthState};

/// Transcript prefix of the Exec handshake (ALPN `smart-explorer/share-exec/2`).
pub const EXEC_TRANSCRIPT_PREFIX: &str = "smart-explorer/share-exec/v2|";

/// The server half of a relation handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationServerHello {
    /// Fixed per protocol and ending in `|`: Exec
    /// `smart-explorer/share-exec/v2|`, screen `smart-explorer/share-screen/v1|`.
    pub transcript_prefix: String,
    pub protocol_version: u16,
    /// Protocol capability; the client must name the same one.
    pub capability: String,
    pub challenge: String,
    pub server_device_id: String,
    pub server_public_key: String,
    pub server_fingerprint: String,
    pub server_node_id: String,
}

/// The client half of a relation handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationClientHello {
    pub protocol_version: u16,
    pub capability: String,
    /// `direct` or `room`.
    pub relation_kind: String,
    /// The host's Direct lookup id, or the room id.
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub client_nonce: String,
    pub proof: String,
}

/// A device whose identity and relation proof verified.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProvenPrincipal {
    pub relation_kind: String,
    pub relation_id: String,
    pub device_id: String,
    /// The name the device gave; not verified.
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
}

/// Why a relation proof failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationRejection {
    /// Protocol fields, pinned identity or proof did not verify. Only this
    /// counts as a failed authentication for the host's penalties.
    Authentication,
    /// A verified device in an inactive relation (Direct offline, room secret
    /// unreadable, stored fingerprint pin inconsistent).
    NotAuthorized,
}

impl RelationRejection {
    pub fn counts_as_failed_authentication(self) -> bool {
        matches!(self, Self::Authentication)
    }
}

impl From<&RelationClientHello> for ProvenPrincipal {
    fn from(hello: &RelationClientHello) -> Self {
        Self {
            relation_kind: hello.relation_kind.clone(),
            relation_id: hello.relation_id.clone(),
            device_id: hello.device_id.clone(),
            device_name: hello.device_name.clone(),
            public_key: hello.public_key.clone(),
            fingerprint: hello.fingerprint.clone(),
            node_id: hello.node_id.clone(),
        }
    }
}

impl From<&ProvenPrincipal> for ExecPrincipal {
    fn from(principal: &ProvenPrincipal) -> Self {
        Self {
            relation_kind: principal.relation_kind.clone(),
            relation_id: principal.relation_id.clone(),
            device_id: principal.device_id.clone(),
            device_name: principal.device_name.clone(),
            public_key: principal.public_key.clone(),
            fingerprint: principal.fingerprint.clone(),
            node_id: principal.node_id.clone(),
        }
    }
}

impl From<&ExecPrincipal> for ProvenPrincipal {
    fn from(principal: &ExecPrincipal) -> Self {
        Self {
            relation_kind: principal.relation_kind.clone(),
            relation_id: principal.relation_id.clone(),
            device_id: principal.device_id.clone(),
            device_name: principal.device_name.clone(),
            public_key: principal.public_key.clone(),
            fingerprint: principal.fingerprint.clone(),
            node_id: principal.node_id.clone(),
        }
    }
}

/// The text the client's proof is an HMAC over: the protocol prefix, the
/// fresh challenge and nonce, both identities and the protocol capability.
pub fn relation_transcript(server: &RelationServerHello, client: &RelationClientHello) -> String {
    format!(
        "{}{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|client-to-server|{}",
        server.transcript_prefix,
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
        server.capability,
    )
}

/// Order: protocol and pinned identity, then the relation and its proof.
/// `remote_node` is the transport's verified node id of the connection.
pub fn prove_relation(
    server: &RelationServerHello,
    client: &RelationClientHello,
    remote_node: &str,
    state: &ShareAuthState,
) -> Result<ProvenPrincipal, RelationRejection> {
    validate_common(server, client, remote_node, state)?;
    let transcript = relation_transcript(server, client);
    // E17: a lookup id that runs out after a Direct-code renewal stays valid
    // for exactly the identities that still hold an accepted grant.
    let retiring = (client.relation_kind == "direct"
        && client.relation_id != state.identity.direct_lookup_id)
        .then(|| retiring_secret(&state.identity.node_id, &client.relation_id))
        .flatten();
    match client.relation_kind.as_str() {
        "direct" if client.relation_id == state.identity.direct_lookup_id || retiring.is_some() => {
            if !state.direct_online {
                return Err(RelationRejection::NotAuthorized);
            }
            let grant = state
                .direct_grants
                .iter()
                .find(|grant| {
                    grant.device_id == client.device_id
                        && grant.public_key == client.public_key
                        && grant.fingerprint == client.fingerprint
                        && grant.pins_node(&client.node_id)
                        && grant.state == DirectGrantState::Accepted
                })
                .ok_or(RelationRejection::Authentication)?;
            // E17: the running-out secret first, then the current one (the
            // order of `session_authorize.rs`).
            let proven = retiring
                .as_ref()
                .is_some_and(|old| verify_hmac(old.as_slice(), &transcript, &client.proof))
                || verify_hmac(state.direct_secret.as_slice(), &transcript, &client.proof);
            if !proven {
                return Err(RelationRejection::Authentication);
            }
            if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
                return Err(RelationRejection::NotAuthorized);
            }
        }
        "room" => {
            let room = state
                .rooms
                .iter()
                .find(|room| room.room_id == client.relation_id && room.auto_join)
                .ok_or(RelationRejection::Authentication)?;
            let member = room
                .members
                .iter()
                .find(|member| {
                    member.device_id == client.device_id
                        && member.public_key == client.public_key
                        && member.fingerprint == client.fingerprint
                        && member.node_id == client.node_id
                        && member.is_admitted()
                })
                .ok_or(RelationRejection::Authentication)?;
            let secret = ShareProfiles::room_secret_checked(room)
                .map_err(|_| RelationRejection::NotAuthorized)?
                .ok_or(RelationRejection::NotAuthorized)?;
            if !verify_hmac(&secret, &transcript, &client.proof) {
                return Err(RelationRejection::Authentication);
            }
            if !fingerprint_matches(&member.public_key, &member.fingerprint) {
                return Err(RelationRejection::NotAuthorized);
            }
        }
        _ => return Err(RelationRejection::Authentication),
    }
    let mut principal = ProvenPrincipal::from(client);
    if retiring.is_some() {
        // E17: the same Direct relation of this host under its current lookup
        // id, so grants, the Exec registry and revocations match it.
        principal.relation_id = state.identity.direct_lookup_id.clone();
    }
    Ok(principal)
}

fn validate_common(
    server: &RelationServerHello,
    client: &RelationClientHello,
    remote_node: &str,
    state: &ShareAuthState,
) -> Result<(), RelationRejection> {
    if client.protocol_version != server.protocol_version
        || client.capability != server.capability
        || client.node_id != remote_node
        || server.server_device_id != state.identity.device_id
        || server.server_public_key != state.identity.public_key
        || server.server_fingerprint != state.identity.fingerprint
        || server.server_node_id != state.identity.node_id
        || server.challenge.len() < 24
        || client.client_nonce.len() < 24
        || !fingerprint_matches(&client.public_key, &client.fingerprint)
    {
        return Err(RelationRejection::Authentication);
    }
    Ok(())
}

/// What a device whose relation proof was refused reads (TCP tunnel, device
/// commands): the possible reasons and the way to the grant, never which
/// check failed or a secret. The peer already proved its key over TLS.
pub fn refusal_message(host: &str) -> String {
    let host = match host.trim() {
        "" => "Das andere Gerät",
        name => name,
    };
    format!(
        "{host} lässt dieses Gerät nicht zu: dort nicht freigegeben, gesperrt, neu zu bestätigen oder Direktzugang ausgeschaltet. Auf {host} unter „Geräte & Freigaben“ dieses Gerät freigeben oder beide Geräte neu koppeln."
    )
}

#[cfg(test)]
#[path = "relation_proof_tests.rs"]
mod tests;
