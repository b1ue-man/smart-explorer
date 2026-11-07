//! Errors of the reciprocal Direct wire (moved out of
//! `direct_reciprocal_wire.rs` unchanged).
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectRepairWireError {
    EntropyUnavailable,
    InvalidRepairId,
    FrameTooLarge,
    InvalidLength,
    Truncated,
    UnsupportedVersion,
    UnknownMessage,
    InvalidIdentity,
    InvalidMaterial,
    InvalidOutcome,
    InvalidField,
    TrailingData,
}

impl fmt::Display for DirectRepairWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EntropyUnavailable => "secure repair id generation failed",
            Self::InvalidRepairId => "invalid repair id",
            Self::FrameTooLarge => "repair frame exceeds its fixed bound",
            Self::InvalidLength => "invalid repair frame length",
            Self::Truncated => "truncated repair frame",
            Self::UnsupportedVersion => "unsupported repair wire version",
            Self::UnknownMessage => "unknown repair message",
            Self::InvalidIdentity => "invalid repair identity",
            Self::InvalidMaterial => "invalid repair relation material",
            Self::InvalidOutcome => "invalid repair persistence outcome",
            Self::InvalidField => "invalid repair field",
            Self::TrailingData => "repair frame contains trailing data",
        })
    }
}

impl std::error::Error for DirectRepairWireError {}
