use serde::{Deserialize, Serialize};

use super::direct_lifecycle_error::{
    require_failure, require_non_decreasing, require_not_before, DirectLifecycleError,
};
use super::direct_protocol::{DirectDecisionKind, DirectRequestId, SignedDirectRequest};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectDeliveryState {
    Queued,
    Sent,
    ServerQueued,
    Delivered,
    Received,
    Failed,
    Expired,
}

impl DirectDeliveryState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::ServerQueued => "server_queued",
            Self::Delivered => "delivered",
            Self::Received => "received",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    fn rank(self) -> Option<u8> {
        match self {
            Self::Queued => Some(0),
            Self::Sent => Some(1),
            Self::ServerQueued => Some(2),
            Self::Delivered => Some(3),
            Self::Received => Some(4),
            Self::Failed | Self::Expired => None,
        }
    }

    fn terminal(self) -> bool {
        self.rank().is_none()
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectDecisionState {
    Pending,
    Accepted,
    Rejected,
    Revoked,
    Failed,
    Expired,
}

impl DirectDecisionState {
    pub fn code(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Revoked => "revoked",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

impl From<DirectDecisionKind> for DirectDecisionState {
    fn from(decision: DirectDecisionKind) -> Self {
        match decision {
            DirectDecisionKind::Accepted => Self::Accepted,
            DirectDecisionKind::Rejected => Self::Rejected,
            DirectDecisionKind::Revoked => Self::Revoked,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectDecisionDeliveryState {
    NotStarted,
    Queued,
    Sent,
    ServerQueued,
    Delivered,
    Received,
    Failed,
    Expired,
}

impl DirectDecisionDeliveryState {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::ServerQueued => "server_queued",
            Self::Delivered => "delivered",
            Self::Received => "received",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    fn rank(self) -> Option<u8> {
        match self {
            Self::NotStarted => Some(0),
            Self::Queued => Some(1),
            Self::Sent => Some(2),
            Self::ServerQueued => Some(3),
            Self::Delivered => Some(4),
            Self::Received => Some(5),
            Self::Failed | Self::Expired => None,
        }
    }

    fn terminal(self) -> bool {
        self.rank().is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectDeliveryStatus {
    pub state: DirectDeliveryState,
    pub changed_at: i64,
    pub failure: Option<DirectFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectDecisionStatus {
    pub state: DirectDecisionState,
    pub revision: u64,
    pub changed_at: i64,
    pub message: Option<String>,
    pub failure: Option<DirectFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectDecisionDeliveryStatus {
    pub state: DirectDecisionDeliveryState,
    pub revision: u64,
    pub changed_at: i64,
    pub failure: Option<DirectFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectRequestRecord {
    pub request: SignedDirectRequest,
    pub delivery: DirectDeliveryStatus,
    pub decision: DirectDecisionStatus,
    pub decision_delivery: DirectDecisionDeliveryStatus,
}

impl DirectRequestRecord {
    pub fn new(request: SignedDirectRequest) -> Self {
        let created_at = request.created_at;
        Self {
            request,
            delivery: DirectDeliveryStatus {
                state: DirectDeliveryState::Queued,
                changed_at: created_at,
                failure: None,
            },
            decision: DirectDecisionStatus {
                state: DirectDecisionState::Pending,
                revision: 0,
                changed_at: created_at,
                message: None,
                failure: None,
            },
            decision_delivery: DirectDecisionDeliveryStatus {
                state: DirectDecisionDeliveryState::NotStarted,
                revision: 0,
                changed_at: created_at,
                failure: None,
            },
        }
    }

    pub fn apply(&mut self, event: DirectLifecycleEvent) -> Result<bool, DirectLifecycleError> {
        if event.request_id() != &self.request.request_id {
            return Err(DirectLifecycleError::WrongRequest);
        }
        match event {
            DirectLifecycleEvent::Delivery {
                state, at, failure, ..
            } => self.advance_delivery(state, at, failure),
            DirectLifecycleEvent::Decision {
                decision,
                revision,
                at,
                message,
                ..
            } => self.record_decision(decision, revision, at, message),
            DirectLifecycleEvent::DecisionTerminated {
                state, at, failure, ..
            } => self.terminate_decision(state, at, failure),
            DirectLifecycleEvent::DecisionDelivery {
                state,
                revision,
                at,
                failure,
                ..
            } => self.advance_decision_delivery(state, revision, at, failure),
        }
    }

    fn advance_delivery(
        &mut self,
        next: DirectDeliveryState,
        at: i64,
        failure: Option<DirectFailure>,
    ) -> Result<bool, DirectLifecycleError> {
        require_failure(next == DirectDeliveryState::Failed, &failure)?;
        self.validate_delivery_time(next, at)?;
        let current = self.delivery.state;
        if current.terminal() || current == DirectDeliveryState::Received {
            return Ok(false);
        }
        if next.terminal() {
            self.delivery = DirectDeliveryStatus {
                state: next,
                changed_at: at,
                failure,
            };
            return Ok(true);
        }
        let current_rank = current.rank().unwrap_or_default();
        let next_rank = next.rank().unwrap_or_default();
        if next_rank <= current_rank {
            return Ok(false);
        }
        require_non_decreasing(at, self.delivery.changed_at)?;
        self.delivery = DirectDeliveryStatus {
            state: next,
            changed_at: at,
            failure: None,
        };
        Ok(true)
    }

    fn record_decision(
        &mut self,
        decision: DirectDecisionKind,
        revision: u64,
        at: i64,
        message: Option<String>,
    ) -> Result<bool, DirectLifecycleError> {
        if revision == 0 {
            return Err(DirectLifecycleError::InvalidDecisionRevision);
        }
        if !matches!(decision, DirectDecisionKind::Revoked) && at > self.request.expires_at {
            return Err(DirectLifecycleError::AfterExpiry);
        }
        require_not_before(at, self.request.created_at)?;
        if revision < self.decision.revision {
            return Ok(false);
        }
        let next = DirectDecisionState::from(decision);
        if revision == self.decision.revision {
            return if next == self.decision.state {
                Ok(false)
            } else {
                Err(DirectLifecycleError::DecisionRevisionConflict)
            };
        }
        require_non_decreasing(at, self.decision.changed_at)?;
        let allowed = match self.decision.state {
            DirectDecisionState::Pending => next != DirectDecisionState::Revoked || revision >= 2,
            DirectDecisionState::Accepted => next == DirectDecisionState::Revoked,
            DirectDecisionState::Rejected
            | DirectDecisionState::Revoked
            | DirectDecisionState::Failed
            | DirectDecisionState::Expired => false,
        };
        if !allowed {
            return Err(DirectLifecycleError::InvalidDecisionTransition);
        }
        self.decision = DirectDecisionStatus {
            state: next,
            revision,
            changed_at: at,
            message,
            failure: None,
        };
        self.decision_delivery = DirectDecisionDeliveryStatus {
            state: DirectDecisionDeliveryState::Queued,
            revision,
            changed_at: at,
            failure: None,
        };
        Ok(true)
    }

    fn terminate_decision(
        &mut self,
        state: DirectDecisionState,
        at: i64,
        failure: Option<DirectFailure>,
    ) -> Result<bool, DirectLifecycleError> {
        if !matches!(
            state,
            DirectDecisionState::Failed | DirectDecisionState::Expired
        ) {
            return Err(DirectLifecycleError::InvalidDecisionTransition);
        }
        require_failure(state == DirectDecisionState::Failed, &failure)?;
        require_not_before(at, self.request.created_at)?;
        if state == DirectDecisionState::Expired && at < self.request.expires_at {
            return Err(DirectLifecycleError::InvalidTimestamp);
        }
        if self.decision.state != DirectDecisionState::Pending {
            return Ok(false);
        }
        self.decision = DirectDecisionStatus {
            state,
            revision: self.decision.revision,
            changed_at: at,
            message: None,
            failure,
        };
        Ok(true)
    }

    fn advance_decision_delivery(
        &mut self,
        next: DirectDecisionDeliveryState,
        revision: u64,
        at: i64,
        failure: Option<DirectFailure>,
    ) -> Result<bool, DirectLifecycleError> {
        if revision < self.decision.revision {
            return Ok(false);
        }
        if revision != self.decision.revision || revision == 0 {
            return Err(DirectLifecycleError::DecisionNotKnown);
        }
        require_failure(next == DirectDecisionDeliveryState::Failed, &failure)?;
        require_not_before(at, self.decision.changed_at)?;
        let current = self.decision_delivery.state;
        if current.terminal() || current == DirectDecisionDeliveryState::Received {
            return Ok(false);
        }
        if next.terminal() {
            self.decision_delivery = DirectDecisionDeliveryStatus {
                state: next,
                revision,
                changed_at: at,
                failure,
            };
            return Ok(true);
        }
        let current_rank = current.rank().unwrap_or_default();
        let next_rank = next.rank().unwrap_or_default();
        if next_rank <= current_rank {
            return Ok(false);
        }
        require_non_decreasing(at, self.decision_delivery.changed_at)?;
        self.decision_delivery = DirectDecisionDeliveryStatus {
            state: next,
            revision,
            changed_at: at,
            failure: None,
        };
        Ok(true)
    }

    fn validate_delivery_time(
        &self,
        state: DirectDeliveryState,
        at: i64,
    ) -> Result<(), DirectLifecycleError> {
        require_not_before(at, self.request.created_at)?;
        if state == DirectDeliveryState::Expired {
            if at < self.request.expires_at {
                return Err(DirectLifecycleError::InvalidTimestamp);
            }
        } else if at > self.request.expires_at {
            return Err(DirectLifecycleError::AfterExpiry);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DirectLifecycleEvent {
    Delivery {
        request_id: DirectRequestId,
        state: DirectDeliveryState,
        at: i64,
        failure: Option<DirectFailure>,
    },
    Decision {
        request_id: DirectRequestId,
        decision: DirectDecisionKind,
        revision: u64,
        at: i64,
        message: Option<String>,
    },
    DecisionTerminated {
        request_id: DirectRequestId,
        state: DirectDecisionState,
        at: i64,
        failure: Option<DirectFailure>,
    },
    DecisionDelivery {
        request_id: DirectRequestId,
        state: DirectDecisionDeliveryState,
        revision: u64,
        at: i64,
        failure: Option<DirectFailure>,
    },
}

impl DirectLifecycleEvent {
    fn request_id(&self) -> &DirectRequestId {
        match self {
            Self::Delivery { request_id, .. }
            | Self::Decision { request_id, .. }
            | Self::DecisionTerminated { request_id, .. }
            | Self::DecisionDelivery { request_id, .. } => request_id,
        }
    }
}
