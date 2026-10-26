//! Private worker request values, distinct from peer selectors.
use super::pixel_convert::FrameSize;
use crate::remote_window::WindowPin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenFamily {
    pub pin: WindowPin,
    pub lifetime: String,
    pub session: String,
    pub control: bool,
    pub wanted: Option<FrameSize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", content = "value", rename_all = "snake_case")]
pub(crate) enum FamilyCommand {
    Snapshot,
    Operation(super::family_protocol::FamilyOperation),
    Input(super::family_protocol::FamilyInput),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", content = "value", rename_all = "snake_case")]
pub(crate) enum FamilyReply {
    Snapshot(Option<crate::remote_window::FamilySnapshot>),
    Operation(super::family_protocol::FamilyOperationResult),
    Input(super::family_protocol::FamilyInputResult),
}
