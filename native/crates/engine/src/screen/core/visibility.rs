//! Visibility reports are scoped to one authorized viewer session.
use super::frame_budget::MAX_RESOURCE_SIDE;
use super::wire::invalid;
use serde::{Deserialize, Serialize};
use std::io;

/// The control channel is bounded to 64 KiB. More fragmented native regions
/// must be reported conservatively as unknown rather than dropping pixels.
pub const MAX_VISIBLE_RECTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityState {
    Visible,
    Hidden,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerVisibility {
    pub geometry_generation: u64,
    pub state: VisibilityState,
    pub minimized: bool,
    pub rects: Vec<VisibleRect>,
    /// The picture size (pixels) the viewer measured `rects` in; absent from
    /// older viewers, which the host reads as its running size (Gemini
    /// 3/23: after a size change the rectangles of the picture still shown
    /// were taken for the new one). Older hosts skip the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<super::pixel_convert::FrameSize>,
}

impl ViewerVisibility {
    pub fn validate(&self) -> io::Result<()> {
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

    pub fn hidden(&self) -> bool {
        self.minimized || self.state == VisibilityState::Hidden
    }

    /// `rects` measured in `size` pixels: scaled from `picture` when the
    /// viewer named another size (starts down, ends up, so nothing visible
    /// gets lost); as sent without either size.
    pub fn rects_in(&self, size: Option<super::pixel_convert::FrameSize>) -> Vec<VisibleRect> {
        let (Some(from), Some(to)) = (self.picture, size) else {
            return self.rects.clone();
        };
        if from == to || from.width == 0 || from.height == 0 {
            return self.rects.clone();
        }
        let scale = |value: u32, to: u32, from: u32, up: bool| {
            let scaled = u64::from(value) * u64::from(to);
            let value = if up {
                scaled.div_ceil(u64::from(from))
            } else {
                scaled / u64::from(from)
            };
            u32::try_from(value.min(u64::from(to))).unwrap_or(to)
        };
        self.rects
            .iter()
            .filter_map(|rect| {
                let x = scale(rect.x, to.width, from.width, false);
                let y = scale(rect.y, to.height, from.height, false);
                let right = scale(
                    rect.x.saturating_add(rect.width),
                    to.width,
                    from.width,
                    true,
                );
                let bottom = scale(
                    rect.y.saturating_add(rect.height),
                    to.height,
                    from.height,
                    true,
                );
                (right > x && bottom > y).then_some(VisibleRect {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                })
            })
            .collect()
    }
}

/// Old buffered frames cannot satisfy this acknowledgement's monotonic floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityAck {
    pub geometry_generation: u64,
    pub min_frame_id: u64,
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::screen::core::pixel_convert::FrameSize;

    /// Gemini 3/23: rectangles measured in the picture the viewer still
    /// shows reach the host's picture of another size scaled, starts down
    /// and ends up; without the viewer's size they go as sent.
    #[test]
    fn review1119_task_visibility_rects_follow_the_picture_size() {
        let mut report = ViewerVisibility {
            geometry_generation: 1,
            state: VisibilityState::Visible,
            minimized: false,
            rects: vec![VisibleRect {
                x: 1,
                y: 0,
                width: 3,
                height: 1080,
            }],
            picture: None,
        };
        let host = Some(FrameSize::new(1280, 720));
        assert_eq!(report.rects_in(host), report.rects);
        report.picture = Some(FrameSize::new(1920, 1080));
        assert_eq!(
            report.rects_in(host),
            [VisibleRect {
                x: 0,
                y: 0,
                width: 3,
                height: 720,
            }]
        );
        assert_eq!(report.rects_in(None), report.rects);
        // Older viewers send no `picture`, older hosts skip it.
        let json = crate::screen::core::wire::to_json(&ViewerVisibility {
            picture: None,
            ..report.clone()
        })
        .unwrap();
        assert!(!String::from_utf8(json).unwrap().contains("picture"));
    }
}
