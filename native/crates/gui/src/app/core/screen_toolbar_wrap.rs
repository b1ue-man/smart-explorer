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
//!
//! Fix 2026-11-19 (K5): an item that reaches past the right edge widens the
//! row's `max_rect` (egui 0.29.1 `layout.rs` 50-53, `Region::
//! expand_to_include_rect`), after which `available_size_before_wrap` and
//! egui's own row break count to the widened edge, so every later item ran
//! past the window. The bar's edges are therefore noted before its first
//! item (`begin`) and every check here counts to them; lists and long texts
//! never reach past them (`fitted` shortens, `note` wraps).
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

fn bounds_id() -> egui::Id {
    egui::Id::new("screen_toolbar_bounds")
}

/// Notes the bar's left and right edge before its first item.
pub(super) fn begin(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let bounds = (rect.left(), rect.right());
    ui.ctx()
        .data_mut(|data| data.insert_temp(bounds_id(), bounds));
}

/// The bar's edges as noted by `begin` (else the row's own).
fn bounds(ui: &egui::Ui) -> (f32, f32) {
    ui.ctx()
        .data(|data| data.get_temp::<(f32, f32)>(bounds_id()))
        .unwrap_or_else(|| (ui.max_rect().left(), ui.max_rect().right()))
}

/// The rest of the current row up to the bar's right edge.
fn rest(ui: &egui::Ui) -> f32 {
    (bounds(ui).1 - ui.cursor().left()).max(0.0)
}

fn editing_id() -> egui::Id {
    egui::Id::new("screen_toolbar_editing")
}

/// A text field of the bar (`response`) that has the keyboard focus keeps
/// it for this pass (Gemini 10, fix 2026-11-19): the steering picture took
/// the focus back in every frame, so digits typed into „Höchste Datenrate“
/// or „Eigene Größe“ went to the host as keys.
pub(super) fn note_editing(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        let pass = ui.ctx().cumulative_pass_nr();
        ui.ctx()
            .data_mut(|data| data.insert_temp(editing_id(), pass));
    }
}

/// Whether a text field of the bar has the keyboard focus in this pass
/// (the bar is drawn before the picture).
pub fn toolbar_editing(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<u64>(editing_id()))
        .is_some_and(|pass| pass == ctx.cumulative_pass_nr())
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
    fitted_in(ui, list, text.into(), true)
}

/// `fitted` for the list of a label placed just before it („Gerät:“,
/// „Quelle:“): `reserve_pair` broke the row before the label already, so
/// the list never breaks away from it (U3); it is shortened to the rest.
pub(super) fn fitted_after_label(
    ui: &mut egui::Ui,
    list: egui::ComboBox,
    text: impl Into<WidgetText>,
) -> egui::ComboBox {
    fitted_in(ui, list, text.into(), false)
}

fn fitted_in(
    ui: &mut egui::Ui,
    list: egui::ComboBox,
    text: WidgetText,
    may_break: bool,
) -> egui::ComboBox {
    // Measured as drawn (a `RichText` may be italic or of another size).
    let galley = text.clone().into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let spacing = ui.spacing();
    let width = galley.size().x
        + TEXT_SLACK
        + spacing.icon_width
        + spacing.icon_spacing
        + 2.0 * spacing.button_padding.x;
    let (left, right) = bounds(ui);
    if may_break {
        reserve(ui, width.min(right - left));
    }
    // What still does not fit (wider than the bar, or the rest right of its
    // label) keeps that width and shortens its text (egui
    // `ComboBox::wrap_mode`), so it never runs past the edge.
    let room = rest(ui);
    let list = if width > room {
        list.wrap_mode(egui::TextWrapMode::Truncate)
    } else {
        list
    };
    list.width(width.min(room)).selected_text(text)
}

/// Starts a new row unless `width` fits the rest of the current one. The
/// rest of the row is `available_size_before_wrap` (egui 0.29.1 `ui.rs`
/// 996): in a wrapping layout `available_width` is the width of a whole row
/// (`layout.rs` 439-445), so the check never broke a row (fix 2026-11-19).
pub(super) fn reserve(ui: &mut egui::Ui, width: f32) {
    let at_row_start = ui.cursor().left() <= bounds(ui).0 + ROW_START_SLACK;
    if breaks_before(Some(width), rest(ui), at_row_start) {
        ui.end_row();
    }
}

/// A text of the bar (a state, a warning) that may be longer than a row: it
/// starts a new row when it does not fit the rest of this one and wraps
/// inside the bar instead of running past its edge (fix 2026-11-19).
pub(super) fn note(ui: &mut egui::Ui, text: impl Into<WidgetText>) {
    let text = text.into();
    let width = text
        .clone()
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Body,
        )
        .size()
        .x;
    let (left, right) = bounds(ui);
    reserve(ui, width.min(right - left));
    ui.add(egui::Label::new(text).wrap());
}

/// The outer width of a button showing `text`. A button inside a child `Ui`
/// (`add_enabled_ui`, `scope`) needs it reserved: the child starts at the
/// cursor, and egui breaks a row only after its start (`layout.rs` 534).
pub(super) fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

/// The outer width of a checkbox with `text` (egui 0.29.1 `checkbox.rs`
/// 59-71: icon, its spacing and the text, at least `interact_size`).
pub(super) fn checkbox_width(ui: &egui::Ui, text: &str) -> f32 {
    let spacing = ui.spacing();
    (spacing.icon_width + spacing.icon_spacing + text_width(ui, text, TextStyle::Body))
        .max(spacing.interact_size.x)
}

/// The outer width of a label showing `text` (body text, unwrapped).
pub(super) fn label_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, TextStyle::Body)
}

/// The outer width of a single-line `TextEdit` of `desired` points (egui
/// 0.29.1 `text_edit/builder.rs` 127, 492-522: the clipped single line is
/// as wide as asked, plus the default margin of 4 points on each side).
pub(super) fn text_edit_width(desired: f32) -> f32 {
    desired + 8.0
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
