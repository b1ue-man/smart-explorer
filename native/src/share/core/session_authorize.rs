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
use super::super::profiles::{fingerprint_matches, ShareProfiles};
use super::super::relation_rights::SessionAuthorization;
use super::super::types::{DirectGrant, RoomProfile, ShareAuthState};
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

    #[cfg(test)]
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
        match hello.relation_kind.as_str() {
            "direct" if hello.relation_id == state.identity.direct_lookup_id => {
                if !state.direct_online {
                    return Err(eio("Direktverbindung ist offline"));
                }
                let grant = state
                    .direct_grants
                    .iter()
                    .find(|g| {
                        g.authorizes_session(&hello.device_id, &hello.public_key, &hello.node_id)
                    })
                    .ok_or_else(|| eio("Direktfreigabe nicht akzeptiert"))?;
                let inputs = ProofInputs {
                    local_device_id: &state.identity.device_id,
                    local_node_id: &state.identity.node_id,
                    public_key: &grant.public_key,
                    fingerprint: &grant.fingerprint,
                    key: KeyRef::Direct(&state.direct_secret),
                };
                if !self.memo.verified(&inputs) {
                    if !fingerprint_matches(&grant.public_key, &grant.fingerprint) {
                        return Err(eio("Direktfreigabe hat ungueltigen Fingerprint"));
                    }
                    let payload = self.payload("direct", state);
                    if !verify_hmac(&state.direct_secret, &payload, &hello.session_proof) {
                        return Err(eio("Session-Proof ungueltig"));
                    }
                    self.memo.remember(&inputs);
                }
                Ok(Granted::Direct(grant))
            }
            "room" => {
                let room = state
                    .rooms
                    .iter()
                    .find(|r| r.room_id == hello.relation_id && r.auto_join)
                    .ok_or_else(|| eio("Unbekannter Raum"))?;
                let member = room
                    .members
                    .iter()
                    .find(|m| {
                        m.authorizes_session(&hello.device_id, &hello.public_key, &hello.node_id)
                    })
                    .ok_or_else(|| eio("Geraet nicht im Raum"))?;
                if member.node_id != hello.node_id || member.public_key != hello.public_key {
                    return Err(eio("Raumgeraet hat Identitaetskonflikt"));
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
                        return Err(eio("Raumgeraet hat ungueltigen Fingerprint"));
                    }
                    let secret = Zeroizing::new(
                        ShareProfiles::room_secret_checked(room)
                            .map_err(eio)?
                            .ok_or_else(|| eio("Raum-Secret fehlt"))?,
                    );
                    let payload = self.payload("room", state);
                    if !verify_hmac(&secret, &payload, &hello.session_proof) {
                        return Err(eio("Session-Proof ungueltig"));
                    }
                    self.memo.remember(&inputs);
                }
                Ok(Granted::Room(room))
            }
            _ => Err(eio("Unbekannte oder nicht autorisierte Relation")),
        }
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

    #[cfg(test)]
    pub(in crate::share) fn memo_is_empty(&self) -> bool {
        self.memo.is_empty()
    }
}

#[cfg(test)]
#[path = "session_authorize_tests.rs"]
mod tests;
