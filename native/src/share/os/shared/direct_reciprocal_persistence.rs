use std::fmt;

use super::core::random_token;
use super::direct_reciprocal::{
    DirectReciprocalApply, DirectReciprocalConflict, DirectReciprocalError, DirectReciprocalPeer,
};
use super::profile_store::{
    credential_matches, delete_credential_verified, prepare_unique_credential, SecretString,
};
use super::profiles::{direct_contact_secret_account, ShareProfiles};

const MAX_RELATION_REBASES: usize = 8;
const TRANSACTION_ABORTED: &str = "reciprocal Direct profile precondition failed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectReciprocalPersistenceOutcome {
    Changed { contact_id: String },
    AlreadyComplete { contact_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectReciprocalPersistenceError {
    Conflict(DirectReciprocalConflict),
    Invalid(DirectReciprocalError),
    Persistence(String),
}

impl fmt::Display for DirectReciprocalPersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(conflict) => {
                DirectReciprocalError::Conflict(conflict.clone()).fmt(formatter)
            }
            Self::Invalid(error) => error.fmt(formatter),
            Self::Persistence(error) => write!(formatter, "reciprocal Direct persistence: {error}"),
        }
    }
}

impl std::error::Error for DirectReciprocalPersistenceError {}

enum TransactionAbort {
    Domain(DirectReciprocalError),
    Credential(DirectReciprocalPersistenceError),
    NeedsPreparedCredential,
    RebasedToExistingContact,
}

struct PreparedCredential {
    account: String,
    armed: bool,
}

impl PreparedCredential {
    fn prepare(account: String, secret: &SecretString) -> Result<Self, String> {
        prepare_unique_credential(&account, secret)?;
        Ok(Self {
            account,
            armed: true,
        })
    }

    fn cleanup(mut self) -> Result<(), String> {
        let result = delete_credential_verified(&self.account);
        if result.is_ok() {
            self.armed = false;
        }
        result
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl fmt::Debug for PreparedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedCredential")
            .field("account", &self.account)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl Drop for PreparedCredential {
    fn drop(&mut self) {
        if self.armed {
            let _ = delete_credential_verified(&self.account);
        }
    }
}

/// Persist both accepted views of an authenticated reciprocal Direct peer.
///
/// A fresh credential is installed before a profile may reference it. If a CAS
/// rebase discovers an existing canonical contact, the profile transaction is
/// aborted without mutation, the temporary credential is removed, and the
/// operation restarts against that contact. Credential preconditions are
/// rechecked against a cloned candidate before each CAS mutation is installed.
pub fn persist_reciprocal_direct_peer(
    default_home: Option<String>,
    peer: &DirectReciprocalPeer,
) -> Result<DirectReciprocalPersistenceOutcome, DirectReciprocalPersistenceError> {
    let generated_contact_id = random_token(10).map_err(|error| {
        DirectReciprocalPersistenceError::Persistence(format!(
            "secure Direct contact id generation failed: {error}"
        ))
    })?;
    let generated_account = direct_contact_secret_account(&generated_contact_id);
    let expected_secret = SecretString::encoded(peer.material().secret());
    let now = super::core::now_secs();
    let mut prepared = None;

    for _ in 0..MAX_RELATION_REBASES {
        let preview = ShareProfiles::load_checked(default_home.clone())
            .map_err(DirectReciprocalPersistenceError::Persistence)?;
        let preview_outcome = evaluate_candidate(&preview, peer, &generated_contact_id, now)
            .map_err(map_domain_error)?;
        let preview_contact_id = apply_contact_id(&preview_outcome);

        if preview_contact_id == generated_contact_id {
            if prepared.is_none() {
                prepared = Some(
                    PreparedCredential::prepare(generated_account.clone(), &expected_secret)
                        .map_err(DirectReciprocalPersistenceError::Persistence)?,
                );
            }
        } else {
            cleanup_prepared(&mut prepared)?;
            require_matching_credential(preview_contact_id, peer, &expected_secret)?;
        }

        let has_prepared = prepared.is_some();
        let mut abort = None;
        let mut applied = DirectReciprocalApply::AlreadyComplete {
            contact_id: preview_contact_id.to_string(),
        };
        let transaction = ShareProfiles::mutate_persisted(default_home.clone(), |profiles| {
            let candidate_outcome =
                match evaluate_candidate(profiles, peer, &generated_contact_id, now) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        abort = Some(TransactionAbort::Domain(error));
                        return Err(TRANSACTION_ABORTED.to_string());
                    }
                };
            let candidate_contact_id = apply_contact_id(&candidate_outcome);
            if candidate_contact_id == generated_contact_id && !has_prepared {
                abort = Some(TransactionAbort::NeedsPreparedCredential);
                return Err(TRANSACTION_ABORTED.to_string());
            }
            if candidate_contact_id != generated_contact_id && has_prepared {
                abort = Some(TransactionAbort::RebasedToExistingContact);
                return Err(TRANSACTION_ABORTED.to_string());
            }
            if let Err(error) =
                require_matching_credential(candidate_contact_id, peer, &expected_secret)
            {
                abort = Some(TransactionAbort::Credential(error));
                return Err(TRANSACTION_ABORTED.to_string());
            }
            let mut candidate = profiles.clone();
            match candidate.apply_reciprocal_direct_peer(peer, &generated_contact_id, now) {
                Ok(outcome) => {
                    *profiles = candidate;
                    applied = outcome;
                    Ok(())
                }
                Err(error) => {
                    abort = Some(TransactionAbort::Domain(error));
                    Err(TRANSACTION_ABORTED.to_string())
                }
            }
        });

        match transaction {
            Ok(_) => {
                if apply_contact_id(&applied) == generated_contact_id {
                    if let Some(credential) = prepared.take() {
                        credential.disarm();
                    }
                }
                return Ok(map_apply_outcome(applied));
            }
            Err(error) => match abort.take() {
                Some(TransactionAbort::NeedsPreparedCredential) => continue,
                Some(TransactionAbort::RebasedToExistingContact) => {
                    cleanup_prepared(&mut prepared)?;
                    continue;
                }
                Some(TransactionAbort::Domain(error)) => {
                    return Err(cleanup_before_error(map_domain_error(error), &mut prepared));
                }
                Some(TransactionAbort::Credential(error)) => {
                    return Err(cleanup_before_error(error, &mut prepared));
                }
                None => {
                    return Err(cleanup_before_error(
                        DirectReciprocalPersistenceError::Persistence(error),
                        &mut prepared,
                    ));
                }
            },
        }
    }

    Err(cleanup_before_error(
        DirectReciprocalPersistenceError::Persistence(
            "reciprocal Direct contact kept changing during persistence".to_string(),
        ),
        &mut prepared,
    ))
}

fn evaluate_candidate(
    profiles: &ShareProfiles,
    peer: &DirectReciprocalPeer,
    generated_contact_id: &str,
    now: i64,
) -> Result<DirectReciprocalApply, DirectReciprocalError> {
    let mut candidate = profiles.clone();
    candidate.apply_reciprocal_direct_peer(peer, generated_contact_id, now)
}

fn require_matching_credential(
    contact_id: &str,
    peer: &DirectReciprocalPeer,
    expected: &SecretString,
) -> Result<(), DirectReciprocalPersistenceError> {
    let account = direct_contact_secret_account(contact_id);
    let matches = credential_matches(&account, expected)
        .map_err(DirectReciprocalPersistenceError::Persistence)?;
    if matches {
        Ok(())
    } else {
        Err(DirectReciprocalPersistenceError::Conflict(
            DirectReciprocalConflict::RelationMaterial {
                device_id: peer.identity().device_id.clone(),
            },
        ))
    }
}

fn apply_contact_id(outcome: &DirectReciprocalApply) -> &str {
    match outcome {
        DirectReciprocalApply::Changed { contact_id }
        | DirectReciprocalApply::AlreadyComplete { contact_id } => contact_id,
    }
}

fn map_apply_outcome(outcome: DirectReciprocalApply) -> DirectReciprocalPersistenceOutcome {
    match outcome {
        DirectReciprocalApply::Changed { contact_id } => {
            DirectReciprocalPersistenceOutcome::Changed { contact_id }
        }
        DirectReciprocalApply::AlreadyComplete { contact_id } => {
            DirectReciprocalPersistenceOutcome::AlreadyComplete { contact_id }
        }
    }
}

fn map_domain_error(error: DirectReciprocalError) -> DirectReciprocalPersistenceError {
    match error {
        DirectReciprocalError::Conflict(conflict) => {
            DirectReciprocalPersistenceError::Conflict(conflict)
        }
        other => DirectReciprocalPersistenceError::Invalid(other),
    }
}

fn cleanup_prepared(
    prepared: &mut Option<PreparedCredential>,
) -> Result<(), DirectReciprocalPersistenceError> {
    let Some(credential) = prepared.take() else {
        return Ok(());
    };
    credential
        .cleanup()
        .map_err(DirectReciprocalPersistenceError::Persistence)
}

fn cleanup_before_error(
    error: DirectReciprocalPersistenceError,
    prepared: &mut Option<PreparedCredential>,
) -> DirectReciprocalPersistenceError {
    match cleanup_prepared(prepared) {
        Ok(()) => error,
        Err(cleanup) => DirectReciprocalPersistenceError::Persistence(format!(
            "{error}; temporary Direct credential cleanup failed: {cleanup}"
        )),
    }
}
