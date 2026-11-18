//! The row break of the viewer's toolbar (user, 2026-11-18: the wrap of the
//! settings bar was not reliable and pushed items past the edge). egui's
//! `horizontal_wrapped` breaks before a widget only by the size the widget
//! asks for before it draws; a closed list whose text is longer than that
//! (`TextWrapMode::Extend`) grows past the right edge, and a label and its
//! list are two widgets, so the label stayed at the end of one row and its
//! list moved to the next. `item` keeps the parts of one setting together
//! and breaks the row by the width the item really took in the last frame.
use std::hash::Hash;

use eframe::egui;

/// Below this a position counts as the start of a row (layout rounding).
const ROW_START_SLACK: f32 = 0.5;

/// Whether an item of `width` goes to a new row: it is wider than the rest
/// of the row and does not already start one (an item wider than the whole
/// bar keeps a row of its own). An item not measured yet stays in the row;
/// its pass is discarded and laid out again at once with the measured
/// width (`item`, egui's `request_discard`), so no shown frame has it past
/// the edge.
pub(super) fn breaks_before(width: Option<f32>, available: f32, at_row_start: bool) -> bool {
    !at_row_start && width.is_some_and(|width| width > available)
}

/// One setting of the toolbar (a label with its list, a switch with its
/// field) as one unit of the row.
pub(super) fn item<R>(ui: &mut egui::Ui, id: impl Hash, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let key = ui.id().with(("screen_toolbar_item", id));
    let known: Option<f32> = ui.data(|data| data.get_temp(key));
    let at_row_start = ui.cursor().left() <= ui.max_rect().left() + ROW_START_SLACK;
    if breaks_before(known, ui.available_width(), at_row_start) {
        ui.end_row();
    }
    let inner = ui.horizontal(add);
    let width = inner.response.rect.width();
    if known.is_none_or(|known| (known - width).abs() > ROW_START_SLACK) {
        ui.data_mut(|data| data.insert_temp(key, width));
        // This pass placed the item without its width (or with an old one):
        // egui discards the pass and runs it again in the same frame
        // (`Options::max_passes`), now with the measured width.
        ui.ctx().request_discard("screen toolbar item measured");
    }
    inner.inner
}

#[cfg(test)]
mod tests {
    use super::breaks_before;

    #[test]
    fn toolbar_wrap_task_item_moves_whole_to_the_next_row() {
        // Wider than the rest of the row: the whole item moves on.
        assert!(breaks_before(Some(180.0), 120.0, false));
        // Fits the rest of the row: it stays.
        assert!(!breaks_before(Some(100.0), 120.0, false));
        // Already at the start of a row: no empty row, even when wider.
        assert!(!breaks_before(Some(900.0), 600.0, true));
        // Not measured yet: placed, measured, and the pass laid out again.
        assert!(!breaks_before(None, 10.0, false));
    }
}
