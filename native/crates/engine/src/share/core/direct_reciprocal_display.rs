//! Text of `DirectReciprocalError` (moved out of `direct_reciprocal.rs`
//! unchanged, so that file stays below its size budget).
use std::fmt;

use super::{DirectReciprocalConflict, DirectReciprocalError, DirectReciprocalPolicyDenied};

impl fmt::Display for DirectReciprocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity(error) => {
                write!(formatter, "invalid authenticated Direct peer: {error}")
            }
            Self::InvalidMaterial(error) => {
                write!(formatter, "invalid Direct relation material: {error}")
            }
            Self::Conflict(DirectReciprocalConflict::ContactIdentity { device_id }) => {
                write!(
                    formatter,
                    "Direct contact identity conflicts for device {device_id}"
                )
            }
            Self::Conflict(DirectReciprocalConflict::GrantIdentity { device_id }) => {
                write!(
                    formatter,
                    "Direct grant identity conflicts for device {device_id}"
                )
            }
            Self::Conflict(DirectReciprocalConflict::RelationMaterial { device_id }) => {
                write!(
                    formatter,
                    "Direct relation material conflicts for device {device_id}"
                )
            }
            Self::PolicyDenied(DirectReciprocalPolicyDenied::ContactIgnored { device_id }) => {
                write!(
                    formatter,
                    "Direct contact is explicitly ignored for device {device_id}"
                )
            }
            Self::PolicyDenied(DirectReciprocalPolicyDenied::GrantIgnored { device_id }) => {
                write!(
                    formatter,
                    "Direct grant is explicitly ignored for device {device_id}"
                )
            }
            Self::PolicyDenied(DirectReciprocalPolicyDenied::PeerRemoved { device_id }) => {
                write!(
                    formatter,
                    "Direct peer {device_id} was removed by the user; pair it again deliberately"
                )
            }
        }
    }
}
