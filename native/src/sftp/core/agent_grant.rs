//! Extra SSH connections for a main login through the SSH agent (G3-10,
//! decided 2026-10-28 by the main agent on the user's instruction "maximal
//! möglicher Datentransferspeed", superseding E13-S15 (i) for the agent).
//!
//! An extra connection signs again with the agent, and the agent may ask a
//! person at every signature (`ssh-add -c` confirmation, the touch of a
//! FIDO2 security key, a smartcard PIN). No such question may appear for a
//! connection the person did not ask for, so an extra connection uses the
//! agent only when the main login showed that this identity signs without a
//! person:
//!
//! - only the identity that logged the main connection in (by fingerprint);
//! - never a security-key identity (`sk-ssh-ed25519@openssh.com`,
//!   `sk-ecdsa-sha2-nistp256@openssh.com`): the token asks for a touch;
//! - only when the main login's signature took at most `QUICK_SIGNATURE`;
//!   a person confirming or typing a PIN takes longer.
//!
//! The record is per `user@host:port` for the process. An extra login whose
//! own signature took longer (the agent changed meanwhile) revokes it, so at
//! most that one question appears and no further extra login uses the agent.
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// A signature of a local agent takes milliseconds (an RSA-4096 key, a
/// forwarded agent over a slow link: a few hundred); a person who confirms
/// or touches needs longer.
pub(super) const QUICK_SIGNATURE: Duration = Duration::from_millis(750);

/// What the main login through the agent showed about its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentGrant {
    /// SHA256 fingerprint of the identity's public key.
    pub(super) fingerprint: String,
    /// It signed without a person: extra connections may use it.
    pub(super) quick: bool,
}

/// Security-key identities ask for a touch at every signature.
pub(super) fn needs_person(algorithm: &Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::SkEd25519 | Algorithm::SkEcdsaSha2NistP256
    )
}

/// Whether a signature of `algorithm` that took `took` came without a
/// person.
pub(super) fn quick(algorithm: &Algorithm, took: Option<Duration>) -> bool {
    !needs_person(algorithm) && took.is_some_and(|took| took <= QUICK_SIGNATURE)
}

/// Fingerprint and algorithm of an agent identity (a certificate counts by
/// the key it certifies).
pub(super) fn identity_facts(identity: &AgentIdentity) -> (String, Algorithm) {
    match identity {
        AgentIdentity::PublicKey { key, .. } => (
            key.fingerprint(HashAlg::default()).to_string(),
            key.algorithm(),
        ),
        AgentIdentity::Certificate { certificate, .. } => (
            certificate
                .public_key()
                .fingerprint(HashAlg::default())
                .to_string(),
            certificate.algorithm(),
        ),
    }
}

/// Grants by `link_rules::account_key`.
#[derive(Default)]
pub(super) struct Grants {
    by_account: HashMap<String, AgentGrant>,
}

impl Grants {
    pub(super) fn record(&mut self, account: &str, grant: AgentGrant) {
        self.by_account.insert(account.to_string(), grant);
    }

    /// The fingerprint an extra login may sign with, if any.
    pub(super) fn usable(&self, account: &str) -> Option<String> {
        self.by_account
            .get(account)
            .filter(|grant| grant.quick)
            .map(|grant| grant.fingerprint.clone())
    }

    pub(super) fn revoke(&mut self, account: &str) {
        if let Some(grant) = self.by_account.get_mut(account) {
            grant.quick = false;
        }
    }
}

fn grants() -> &'static Mutex<Grants> {
    static GRANTS: OnceLock<Mutex<Grants>> = OnceLock::new();
    GRANTS.get_or_init(|| Mutex::new(Grants::default()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The main login through the agent succeeded with this identity.
pub(super) fn record(account: &str, grant: AgentGrant) {
    lock(grants()).record(account, grant);
}

pub(super) fn usable(account: &str) -> Option<String> {
    lock(grants()).usable(account)
}

/// An extra login's signature needed a person: no further extra login of
/// this account uses the agent.
pub(super) fn revoke(account: &str) {
    lock(grants()).revoke(account);
}

/// A signer that measures how long the agent took for its signatures (the
/// slowest one counts).
pub(super) struct TimedSigner<'a, S> {
    inner: &'a mut S,
    pub(super) took: Option<Duration>,
}

impl<'a, S> TimedSigner<'a, S> {
    pub(super) fn new(inner: &'a mut S) -> Self {
        Self { inner, took: None }
    }
}

impl<S> russh::Signer for TimedSigner<'_, S>
where
    S: russh::Signer + Send,
{
    type Error = S::Error;

    #[allow(clippy::manual_async_fn)]
    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        async move {
            let started = Instant::now();
            let signed = self.inner.auth_sign(key, hash_alg, to_sign).await;
            let took = started.elapsed();
            self.took = Some(self.took.map_or(took, |before| before.max(took)));
            signed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{quick, AgentGrant, Grants, QUICK_SIGNATURE};
    use russh::keys::Algorithm;
    use std::time::Duration;

    #[test]
    fn speed_task_sftp_agent_extra_login_only_for_a_quick_plain_identity() {
        let fast = Some(Duration::from_millis(5));
        assert!(quick(&Algorithm::Ed25519, fast));
        assert!(quick(&Algorithm::Ed25519, Some(QUICK_SIGNATURE)));
        // A person confirming (ssh-add -c) or typing a PIN takes longer.
        assert!(!quick(&Algorithm::Ed25519, Some(Duration::from_secs(2))));
        // Nothing measured: never.
        assert!(!quick(&Algorithm::Ed25519, None));
        // Security keys ask for a touch at every signature, however fast.
        assert!(!quick(&Algorithm::SkEd25519, fast));
        assert!(!quick(&Algorithm::SkEcdsaSha2NistP256, fast));
    }

    #[test]
    fn speed_task_sftp_agent_grant_is_per_account_and_revocable() {
        let mut grants = Grants::default();
        assert_eq!(grants.usable("alice@nas:22"), None);
        grants.record(
            "alice@nas:22",
            AgentGrant {
                fingerprint: "SHA256:a".into(),
                quick: true,
            },
        );
        grants.record(
            "bob@nas:22",
            AgentGrant {
                fingerprint: "SHA256:b".into(),
                quick: false,
            },
        );
        assert_eq!(grants.usable("alice@nas:22").as_deref(), Some("SHA256:a"));
        assert_eq!(grants.usable("bob@nas:22"), None, "slow main signature");
        assert_eq!(grants.usable("alice@nas:2222"), None, "other port");
        // An extra login that needed a person ends the grant.
        grants.revoke("alice@nas:22");
        assert_eq!(grants.usable("alice@nas:22"), None);
    }
}
