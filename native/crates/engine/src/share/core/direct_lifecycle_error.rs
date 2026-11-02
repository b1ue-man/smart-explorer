use std::fmt;

use super::direct_lifecycle::DirectFailure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectLifecycleError {
    WrongRequest,
    InvalidTimestamp,
    AfterExpiry,
    MissingFailure,
    UnexpectedFailure,
    InvalidDecisionRevision,
    DecisionRevisionConflict,
    InvalidDecisionTransition,
    DecisionNotKnown,
}

impl DirectLifecycleError {
    pub fn code(self) -> &'static str {
        match self {
            Self::WrongRequest => "wrong_request",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::AfterExpiry => "after_expiry",
            Self::MissingFailure => "missing_failure",
            Self::UnexpectedFailure => "unexpected_failure",
            Self::InvalidDecisionRevision => "invalid_decision_revision",
            Self::DecisionRevisionConflict => "decision_revision_conflict",
            Self::InvalidDecisionTransition => "invalid_decision_transition",
            Self::DecisionNotKnown => "decision_not_known",
        }
    }
}

impl fmt::Display for DirectLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DirectLifecycleError {}

pub(super) fn require_failure(
    required: bool,
    failure: &Option<DirectFailure>,
) -> Result<(), DirectLifecycleError> {
    match (required, failure.is_some()) {
        (true, false) => Err(DirectLifecycleError::MissingFailure),
        (false, true) => Err(DirectLifecycleError::UnexpectedFailure),
        _ => Ok(()),
    }
}

pub(super) fn require_not_before(at: i64, earliest: i64) -> Result<(), DirectLifecycleError> {
    if at < earliest {
        Err(DirectLifecycleError::InvalidTimestamp)
    } else {
        Ok(())
    }
}

pub(super) fn require_non_decreasing(at: i64, previous: i64) -> Result<(), DirectLifecycleError> {
    require_not_before(at, previous)
}
