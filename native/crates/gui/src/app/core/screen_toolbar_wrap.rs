//! The row break of the viewer's toolbar (user, 2026-11-18: the wrap of the
//! settings bar was not reliable and pushed items past the edge). egui's
//! `horizontal_wrapped` breaks before a widget by the size the widget asks
//! for before it draws. A closed list asked for egui's default list width
//! and then grew with its text (`TextWrapMode::Extend`) past the right edge,
//! and a label and its list are two widgets, so the label stayed at the end
//! of one row and its list moved to the next. Every list of the toolbar
//! therefore asks for the width of its text (`fitted`), and a label with its
//! list reserves both widths before either is placed (`reserve_pair`): egui
//! knows each size before it places it, in the same pass.
use std::hash::Hash;

use eframe::egui::{self, TextStyle, WidgetText};

/// Below this a position counts as the start of a row (layout rounding).
const ROW_START_SLACK: f32 = 0.5;
/// Added to a measured text: rounding of glyph positions and the frame.
const TEXT_SLACK: f32 = 8.0;

/// Whether an item of `width` goes to a new row: it is wider than the rest
/// of the row and does not already start one (an item wider than the whole
/// bar keeps a row of its own).
pub(super) fn breaks_before(width: Option<f32>, available: f32, at_row_start: bool) -> bool {
    !at_row_start && width.is_some_and(|width| width > available)
}

/// The width of `text` in `style`, without wrapping.
fn text_width(ui: &egui::Ui, text: &str, style: TextStyle) -> f32 {
    let font = style.resolve(ui.style());
    let width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    });
    width + TEXT_SLACK
}

/// The outer width of a closed list showing `text`: the text, the arrow and
/// the button's padding.
fn list_width(ui: &egui::Ui, text: &str) -> f32 {
    let spacing = ui.spacing();
    text_width(ui, text, TextStyle::Button)
        + spacing.icon_width
        + spacing.icon_spacing
        + 2.0 * spacing.button_padding.x
}

/// A list of the toolbar that asks for the width of its text and starts a
/// new row itself when that width does not fit the rest of the row: egui's
/// wrapping layout breaks before a button or a switch, but not before a
/// closed list (seen on the user's screen 2026-11-18, the first row ran past
/// the window at „Codec“). The width is in points, so the window's size and
/// the display scaling are already in it.
pub(super) fn fitted(
    ui: &mut egui::Ui,
    list: egui::ComboBox,
    text: impl Into<WidgetText>,
) -> egui::ComboBox {
    let text = text.into();
    let width = list_width(ui, text.text());
    // A list wider than the whole bar keeps the bar's width and shortens
    // its text (egui `ComboBox::wrap_mode`), so it never runs past the edge.
    let full = ui.max_rect().width();
    let list = if width > full {
        list.wrap_mode(egui::TextWrapMode::Truncate)
    } else {
        list
    };
    let width = width.min(full);
    reserve(ui, width);
    list.width(width).selected_text(text)
}

/// Starts a new row unless `width` fits the rest of the current one. The
/// rest of the row is `available_size_before_wrap` (egui 0.29.1 `ui.rs`
/// 996): in a wrapping layout `available_width` is the width of a whole row
/// (`layout.rs` 439-445), so the check never broke a row (fix 2026-11-19).
pub(super) fn reserve(ui: &mut egui::Ui, width: f32) {
    let at_row_start = ui.cursor().left() <= ui.max_rect().left() + ROW_START_SLACK;
    let rest = ui.available_size_before_wrap().x;
    if breaks_before(Some(width), rest, at_row_start) {
        ui.end_row();
    }
}

/// The outer width of a button showing `text`. A button inside a child `Ui`
/// (`add_enabled_ui`, `scope`) needs it reserved: the child starts at the
/// cursor, and egui breaks a row only after its start (`layout.rs` 534).
pub(super) fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

/// A label and the list that belongs to it stay in one row: the row breaks
/// before the label when both do not fit the rest of it.
pub(super) fn reserve_pair(ui: &mut egui::Ui, label: &str, list_text: &str) {
    let width = text_width(ui, label, TextStyle::Body)
        + ui.spacing().item_spacing.x
        + list_width(ui, list_text);
    reserve(ui, width);
}

/// One setting of the toolbar. Its widgets are placed in the wrapping row
/// directly, each with its size known beforehand (lists through `fitted`).
pub(super) fn item<R>(
    ui: &mut egui::Ui,
    _id: impl Hash,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    add(ui)
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
        // Nothing to measure: it stays where it is.
        assert!(!breaks_before(None, 10.0, false));
    }
}
