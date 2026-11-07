//! Authorization of one incoming session against the current auth state
//! (Z-H1). Every check still runs under the `auth` lock and still finds the
//! grant, room and member in the *current* state, so a revocation, a removed
//! member, a room that stopped auto-joining or `direct_online` going false
//! fails the very next check.
//!
//! Only the pure, expensive parts are remembered per session: the SHA-256
//! fingerprint check, the HMAC of the session proof and, for rooms, the read
//! of the room secret from the credential store. The memo is validated by
//! value, never by a counter that a mutation might forget to bump: a later
//! check reuses it only when every input of that verification is equal to
//! what it was (local device and node id, the peer's key and fingerprint,
//! and the Direct secret bytes or the room's local profile id). Any change
//! of one of them recomputes in full, exactly as before.
//!
//! A room secret is keyed by the room's random local profile id and is
//! written once: `ShareProfiles` stores it under a fresh id and refuses a
//! different secret for an existing room (`finish_rebased_credential`), and
//! leaving a room removes the profile (and with it the room from the state)
//! together with the secret. The same profile id therefore always verifies
//! with the same secret.
use std::io;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::super::core::{eio, verify_hmac};
use super::super::identity::code_renewal_registry::retiring_secret;
use super::super::profiles::{fingerprint_matches, ShareProfiles};
use super::super::relation_rights::SessionAuthorization;
use super::super::types::{DirectGrant, DirectGrantState, RoomProfile, ShareAuthState};
use super::denial::{denied, SessionDenial};
use super::{session_payload, IncomingSession};

/// The key material one proof verification used.
enum ProofKey {
    /// Direct: the exact relation secret the HMAC was verified with.
    Direct(Zeroizing<Vec<u8>>),
    /// Room: the local profile id whose write-once credential verified.
    Room(String),
}

enum KeyRef<'a> {
    Direct(&'a [u8]),
    Room(&'a str),
}

/// Everything a successful fingerprint and proof check depended on.
struct ProofMemo {
    local_device_id: String,
    local_node_id: String,
    public_key: String,
    fingerprint: String,
    key: ProofKey,
}

struct ProofInputs<'a> {
    local_device_id: &'a str,
    local_node_id: &'a str,
    public_key: &'a str,
    fingerprint: &'a str,
    key: KeyRef<'a>,
}

impl ProofMemo {
    fn matches(&self, inputs: &ProofInputs<'_>) -> bool {
        self.local_device_id == inputs.local_device_id
            && self.local_node_id == inputs.local_node_id
            && self.public_key == inputs.public_key
            && self.fingerprint == inputs.fingerprint
            && match (&self.key, &inputs.key) {
                (ProofKey::Direct(memo), KeyRef::Direct(current)) => memo.as_slice() == *current,
                (ProofKey::Room(memo), KeyRef::Room(current)) => memo.as_str() == *current,
                _ => false,
            }
    }

    fn of(inputs: &ProofInputs<'_>) -> Self {
        Self {
            local_device_id: inputs.local_device_id.to_string(),
            local_node_id: inputs.local_node_id.to_string(),
            public_key: inputs.public_key.to_string(),
            fingerprint: inputs.fingerprint.to_string(),
            key: match inputs.key {
                KeyRef::Direct(secret) => ProofKey::Direct(Zeroizing::new(secret.to_vec())),
                KeyRef::Room(id) => ProofKey::Room(id.to_string()),
            },
        }
    }
}

/// The verified proof of one session (shared by its clones).
#[derive(Default)]
pub(in crate::share) struct AuthMemo(Mutex<Option<ProofMemo>>);

impl std::fmt::Debug for AuthMemo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuthMemo([REDACTED])")
    }
}

impl AuthMemo {
    fn verified(&self, inputs: &ProofInputs<'_>) -> bool {
        // A poisoned memo is no proof: the full check runs.
        self.0
            .lock()
            .ok()
            .is_some_and(|memo| memo.as_ref().is_some_and(|memo| memo.matches(inputs)))
    }

    fn remember(&self, inputs: &ProofInputs<'_>) {
        if let Ok(mut memo) = self.0.lock() {
            *memo = Some(ProofMemo::of(inputs));
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::share) fn is_empty(&self) -> bool {
        self.0.lock().map(|memo| memo.is_none()).unwrap_or(true)
    }
}

/// What the current state grants this session.
enum Granted<'s> {
    Direct(&'s DirectGrant),
    Room(&'s RoomProfile),
}

impl IncomingSession {
    /// Re-evaluates the identity, grant, relation secret, and export policy
    /// for every accepted filesystem stream. The QUIC connection alone is
    /// never an authorization cache.
    pub(in crate::share) fn authorize_state(
        &self,
        state: &ShareAuthState,
    ) -> io::Result<SessionAuthorization> {
        Ok(match self.granted(state)? {
            Granted::Direct(grant) => {
                SessionAuthorization::direct(state.default_direct_exports.clone(), grant)
            }
            Granted::Room(room) => SessionAuthorization::room(room),
        })
    }

    /// `authorize`, without copying the export table: whether the relation
    /// may write (the per-block check of a running transfer).
    pub(in crate::share) fn may_write(
        &self,
        auth: &Arc<Mutex<ShareAuthState>>,
    ) -> io::Result<bool> {
        let state = auth.lock().map_err(|_| eio("Share-Auth gesperrt"))?;
        Ok(match self.granted(&state)? {
            Granted::Direct(grant) => grant.write,
            Granted::Room(room) => room.policy.members_may_write,
        })
    }

    fn granted<'s>(&self, state: &'s ShareAuthState) -> io::Result<Granted<'s>> {
        let hello = &self.hello;
        // E17: a lookup id that runs out after a Direct-code renewal stays
        // valid for exactly the identities that still hold a grant.
        let retiring = (hello.relation_kind == "direct"
            && hello.relation_id != state.identity.direct_lookup_id)
            .then(|| retiring_secret(&state.identity.node_id, &hello.relation_id))
            .flatten();
        match hello.relation_kind.as_str() {
            "direct"
                if hello.relation_id == state.identity.direct_lookup_id || retiring.is_some() =>
            {
                Ok(Granted::Direct(self.granted_direct(
                    state,
                    retiring.as_ref().map(|secret| secret.as_slice()),
                )?))
            }
            "room" => {
                let room = state
                    .rooms
                    .iter()
                    .find(|r| r.room_id == hello.relation_id && r.auto_join)
                    .ok_or_else(|| denied(SessionDenial::RoomUnknown, "Unbekannter Raum"))?;
                let member = room
                    .members
                    .iter()
                    .find(|m| {
                        m.authorizes_session(&hello.device_id, &hello.public_key, &hello.node_id)
                    })
                    .ok_or_else(|| denied(SessionDenial::RoomAdmission, "Geraet nicht im Raum"))?;
                if member.node_id != hello.node_id || member.public_key != hello.public_key {
                    return Err(denied(
                        SessionDenial::Identity,
                        "Raumgeraet hat Identitaetskonflikt",
                    ));
                }
                let inputs = ProofInputs {
                    local_device_id: &state.identity.device_id,
                    local_node_id: &state.identity.node_id,
                    public_key: &member.public_key,
                    fingerprint: &member.fingerprint,
                    key: KeyRef::Room(&room.id),
                };
                if !self.memo.verified(&inputs) {
                    if !fingerprint_matches(&member.public_key, &member.fingerprint) {
                        return Err(denied(
                            SessionDenial::Identity,
                            "Raumgeraet hat ungueltigen Fingerprint",
                        ));
                    }
                    let secret = Zeroizing::new(
                        ShareProfiles::room_secret_checked(room)
                            .map_err(eio)?
                            .ok_or_else(|| eio("Raum-Secret fehlt"))?,
                    );
                    let payload = self.payload("room", state);
                    if !verify_hmac(&secret, &payload, &hello.session_proof) {
                        return Err(denied(
                            SessionDenial::Authentication,
                            "Session-Proof ungueltig",
                        ));
                    }
                    self.memo.remember(&inputs);
                }
                Ok(Granted::Room(room))
            }
            // A Direct hello naming another lookup id than this host's
            // current one: the peer kept a code this host replaced.
            "direct" => Err(denied(
                SessionDenial::CodeChanged,
                "Unbekannte oder nicht autorisierte Relation",
            )),
            _ => Err(denied(
                SessionDenial::Authentication,
                "Unbekannte oder nicht autorisierte Relation",
            )),
        }
    }

    /// The Direct grant of this hello. With `retiring`, the proof may use
    /// the retiring secret or the current one: a peer that stored the new
    /// secret but not yet the new lookup id (interrupted renewal) still
    /// passes and completes its renewal.
    fn granted_direct<'s>(
        &self,
        state: &'s ShareAuthState,
        retiring: Option<&[u8]>,
    ) -> io::Result<&'s DirectGrant> {
        let hello = &self.hello;
        if !state.direct_online {
            return Err(denied(
                SessionDenial::Offline,
                "Direktverbindung ist offline",
            ));
        }
        let grant = state
            .direct_grants
            .iter()
            .find(|g| g.authorizes_session(&hello.device_id, &hello.public_key, &hello.node_id))
            .ok_or_else(|| {
                denied(
                    direct_denial(state, hello),
                    "Direktfreigabe nicht akzeptiert",
                )
            })?;
        let keys: [Option<&[u8]>; 2] = match retiring {
            Some(old) => [Some(old), Some(state.direct_secret.as_slice())],
            None => [Some(state.direct_secret.as_slice()), None],
        };
        if keys
            .into_iter()
            .flatten()
            .any(|key| self.memo.verified(&direct_inputs(state, grant, key)))
        {
            return Ok(grant);
        }
        if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
            return Err(denied(
                SessionDenial::Identity,
                "Direktfreigabe hat ungueltigen Fingerprint",
            ));
        }
        let payload = self.payload("direct", state);
        for key in keys.into_iter().flatten() {
            if verify_hmac(key, &payload, &hello.session_proof) {
                self.memo.remember(&direct_inputs(state, grant, key));
                return Ok(grant);
            }
        }
        Err(denied(
            SessionDenial::Authentication,
            "Session-Proof ungueltig",
        ))
    }

    fn payload(&self, kind: &str, state: &ShareAuthState) -> String {
        let hello = &self.hello;
        session_payload(
            kind,
            &hello.relation_id,
            &hello.device_id,
            &state.identity.device_id,
            &hello.node_id,
            &state.identity.node_id,
            &hello.session_nonce,
        )
    }

    /// E17: the lookup id a Direct hello named (renewal bookkeeping of the
    /// reciprocal stream, `direct_reciprocal_renewal.rs`).
    pub(in crate::share) fn direct_relation_id(&self) -> Option<&str> {
        (self.hello.relation_kind == "direct").then_some(self.hello.relation_id.as_str())
    }

    /// `(device_id, public_key, node_id)` of this session; the node id is
    /// the TLS-verified one (`authenticate_incoming_session`).
    pub(in crate::share) fn peer_pins(&self) -> (&str, &str, &str) {
        (
            &self.hello.device_id,
            &self.hello.public_key,
            &self.hello.node_id,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::share) fn memo_is_empty(&self) -> bool {
        self.memo.is_empty()
    }
}

fn direct_inputs<'a>(
    state: &'a ShareAuthState,
    grant: &'a DirectGrant,
    key: &'a [u8],
) -> ProofInputs<'a> {
    ProofInputs {
        local_device_id: &state.identity.device_id,
        local_node_id: &state.identity.node_id,
        public_key: &grant.public_key,
        fingerprint: &grant.fingerprint,
        key: KeyRef::Direct(key),
    }
}

/// Why no grant authorizes this Direct hello: a grant of exactly this
/// identity that is suspended or blocked, otherwise none at all. Only the
/// state of the peer's own grant is named; the peer proved this key over TLS.
fn direct_denial(state: &ShareAuthState, hello: &super::super::wire::PeerHello) -> SessionDenial {
    let own = state.direct_grants.iter().find(|grant| {
        grant.device_id == hello.device_id
            && grant.public_key == hello.public_key
            && grant.pins_node(&hello.node_id)
    });
    match own.map(|grant| &grant.state) {
        Some(DirectGrantState::Reconfirm) => SessionDenial::Reconfirm,
        Some(DirectGrantState::Ignored) => SessionDenial::Blocked,
        Some(DirectGrantState::Accepted) => SessionDenial::Identity,
        None => SessionDenial::NotGranted,
    }
}

#[cfg(test)]
#[path = "session_authorize_tests.rs"]
mod tests;
