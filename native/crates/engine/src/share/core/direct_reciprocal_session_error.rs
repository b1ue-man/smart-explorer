//! Errors of the reciprocal Direct session (moved out of
//! `direct_reciprocal_session.rs` unchanged).
use std::fmt;

use super::super::direct_reciprocal_store::DirectRepairStoreError;
use super::super::direct_reciprocal_wire::DirectRepairWireError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectRepairSessionError {
    PolicyDenied,
    CapabilityNotRequested,
    WrongAuthorizationDirection,
    InvalidAuthenticatedBinding,
    InvalidLocalIdentity,
    InvalidPresentedIdentity,
    IdentityConflict,
    RelationMaterialConflict,
    InvalidRelationMaterial,
    SelfRelation,
    RepairIdMismatch,
    TranscriptMismatch,
    InvalidStoreReceipt,
    Store(DirectRepairStoreError),
    Wire(DirectRepairWireError),
}

impl From<DirectRepairStoreError> for DirectRepairSessionError {
    fn from(error: DirectRepairStoreError) -> Self {
        Self::Store(error)
    }
}

impl fmt::Display for DirectRepairSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PolicyDenied => "local Direct policy explicitly denies reciprocal repair",
            Self::CapabilityNotRequested => "reciprocal Direct capability was not requested",
            Self::WrongAuthorizationDirection => "Direct authorization has the wrong direction",
            Self::InvalidAuthenticatedBinding => "authenticated Direct binding is invalid",
            Self::InvalidLocalIdentity => "local Direct identity is invalid",
            Self::InvalidPresentedIdentity => "presented Direct identity is invalid",
            Self::IdentityConflict => "presented Direct identity conflicts with authenticated pins",
            Self::RelationMaterialConflict => "Direct relation material conflicts with saved state",
            Self::InvalidRelationMaterial => "presented Direct relation material is invalid",
            Self::SelfRelation => "reciprocal Direct repair cannot target the local identity",
            Self::RepairIdMismatch => "reciprocal Direct repair id does not match",
            Self::TranscriptMismatch => "reciprocal Direct transcript does not match",
            Self::InvalidStoreReceipt => "durable Direct store receipt does not match",
            Self::Store(DirectRepairStoreError::Retryable) => "durable store should be retried",
            Self::Store(DirectRepairStoreError::PolicyDenied) => "local policy denied store",
            Self::Store(DirectRepairStoreError::RelationConflict) => "relation conflict in store",
            Self::Store(DirectRepairStoreError::ReplayConflict) => "replay conflict in store",
            Self::Store(DirectRepairStoreError::StaleLocalIdentity) => "local identity changed",
            Self::Store(DirectRepairStoreError::Unavailable) => "durable store is unavailable",
            Self::Wire(error) => return fmt::Display::fmt(error, formatter),
        })
    }
}

impl std::error::Error for DirectRepairSessionError {}
