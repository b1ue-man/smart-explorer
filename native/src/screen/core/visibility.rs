//! Visibility reports are scoped to one authorized viewer session.
use super::frame_budget::MAX_RESOURCE_SIDE;
use super::wire::invalid;
use serde::{Deserialize, Serialize};
use std::io;

/// The control channel is bounded to 64 KiB. More fragmented native regions
/// must be reported conservatively as unknown rather than dropping pixels.
pub(crate) const MAX_VISIBLE_RECTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VisibilityState {
    Visible,
    Hidden,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VisibleRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewerVisibility {
    pub geometry_generation: u64,
    pub state: VisibilityState,
    pub minimized: bool,
    pub rects: Vec<VisibleRect>,
}

impl ViewerVisibility {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.geometry_generation == 0 || self.rects.len() > MAX_VISIBLE_RECTS {
            return Err(invalid(
                "invalid viewer visibility generation or region count",
            ));
        }
        for rect in &self.rects {
            if rect.width == 0
                || rect.height == 0
                || rect
                    .x
                    .checked_add(rect.width)
                    .is_none_or(|x| x > MAX_RESOURCE_SIDE)
                || rect
                    .y
                    .checked_add(rect.height)
                    .is_none_or(|y| y > MAX_RESOURCE_SIDE)
            {
                return Err(invalid("invalid viewer visibility rectangle"));
            }
        }
        Ok(())
    }

    pub(crate) fn hidden(&self) -> bool {
        self.minimized || self.state == VisibilityState::Hidden
    }
}

/// Old buffered frames cannot satisfy this acknowledgement's monotonic floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VisibilityAck {
    pub geometry_generation: u64,
    pub min_frame_id: u64,
}
