//! Window "Lizenzen von Drittanbietern" (plan C4): the license texts of every
//! Rust crate embedded in this program, with a search. Einstellungen →
//! Updates opens it from the button below the version line. The window keeps
//! its state in egui's temporary memory, so `App` needs no field;
//! [`show_licenses_window`] runs every frame next to the settings window.

use std::sync::{Arc, OnceLock};

use super::prelude::{egui, RichText};
use super::theme;
use crate::licenses;

const TITLE: &str = "Lizenzen von Drittanbietern";

#[derive(Clone, Default)]
struct LicensesWindow {
    open: bool,
    query: String,
    /// Numbers of the lines that contain `query`, ignoring case.
    matches: Arc<Vec<usize>>,
    /// Position in `matches` of the highlighted line.
    current: Option<usize>,
    /// Line to scroll to in this frame.
    scroll_to: Option<usize>,
}

fn state_id() -> egui::Id {
    egui::Id::new("se_crate_licenses_window")
}

/// Lines of the embedded file, or the notice of a build without it.
fn lines() -> &'static [&'static str] {
    static LINES: OnceLock<Vec<&'static str>> = OnceLock::new();
    LINES.get_or_init(|| {
        licenses::text()
            .unwrap_or(licenses::MISSING_NOTICE)
            .lines()
            .collect()
    })
}

/// Lowercase copy of every line for the search, built on first use.
fn lowercase_lines() -> &'static [String] {
    static LOWER: OnceLock<Vec<String>> = OnceLock::new();
    LOWER.get_or_init(|| lines().iter().map(|line| line.to_lowercase()).collect())
}

fn summary() -> &'static str {
    static SUMMARY: OnceLock<String> = OnceLock::new();
    SUMMARY.get_or_init(|| match licenses::sha256() {
        Some(sha256) => format!(
            "{} Rust-Bibliotheken · SHA-256 {}…",
            licenses::crates().len(),
            sha256.get(..16).unwrap_or(&sha256)
        ),
        None => licenses::MISSING_NOTICE.to_string(),
    })
}

/// Button "Lizenzen von Drittanbietern…" below the version line of
/// Einstellungen → Updates.
pub(super) fn licenses_button(ui: &mut egui::Ui) {
    if ui
        .button("Lizenzen von Drittanbietern…")
        .on_hover_text("Lizenztexte aller Rust-Bibliotheken, die in diesem Programm stecken")
        .clicked()
    {
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<LicensesWindow>(state_id())
                .open = true;
        });
    }
}

/// Draws the window while it is open; call once per frame.
pub(super) fn show_licenses_window(ctx: &egui::Context) {
    let id = state_id();
    let Some(mut state) = ctx.data_mut(|data| data.get_temp::<LicensesWindow>(id)) else {
        return;
    };
    if !state.open {
        return;
    }
    let mut open = true;
    super::dialog_geometry::window(ctx, "licenses_ui.0", TITLE)
        .id(egui::Id::new(TITLE))
        .open(&mut open)
        .default_size([720.0, 520.0])
        .max_size(theme::window_content_limit(ctx))
        .constrain_to(ctx.screen_rect().shrink(16.0))
        .show(ctx, |ui| window_body(ui, &mut state));
    state.open = open;
    ctx.data_mut(|data| data.insert_temp(id, state));
}

fn window_body(ui: &mut egui::Ui, state: &mut LicensesWindow) {
    if licenses::text().is_some() {
        ui.label(RichText::new(summary()).color(theme::muted(ui)));
    } else {
        let warn = ui.visuals().warn_fg_color;
        ui.colored_label(warn, summary());
    }
    ui.horizontal(|ui| search_bar(ui, state));
    ui.separator();

    let lines = lines();
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let mut area = egui::ScrollArea::both()
        .min_scrolled_height(0.0)
        .max_height(super::dialog_geometry::remaining_height(ui, 0.0))
        .id_salt("se_crate_licenses_text")
        .auto_shrink([false, false]);
    if let Some(row) = state.scroll_to.take() {
        let pitch = row_height + ui.spacing().item_spacing.y;
        // A few lines of context above the match.
        area = area.vertical_scroll_offset((row.saturating_sub(3) as f32) * pitch);
    }
    let highlighted = state
        .current
        .and_then(|index| state.matches.get(index).copied());
    let highlight = ui.visuals().selection.bg_fill;
    area.show_rows(ui, row_height, lines.len(), |ui, rows| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for row in rows {
            let mut text = RichText::new(lines[row]).monospace();
            if highlighted == Some(row) {
                text = text.background_color(highlight);
            }
            ui.label(text);
        }
    });
}

fn search_bar(ui: &mut egui::Ui, state: &mut LicensesWindow) {
    let response = ui.add(
        egui::TextEdit::singleline(&mut state.query)
            .hint_text("Suchen…")
            .desired_width(260.0),
    );
    if response.changed() {
        state.matches = Arc::new(find_matches(&state.query));
        state.current = None;
    }
    let entered = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if entered {
        response.request_focus();
    }
    let count = state.matches.len();
    if !state.query.trim().is_empty() {
        ui.label(match (count, state.current) {
            (0, _) => "Keine Treffer".to_string(),
            (_, Some(current)) => format!("Treffer {} von {count}", current + 1),
            (_, None) => format!("{count} Treffer"),
        });
    }
    if ui
        .add_enabled(count > 0, egui::Button::new("Vorheriger"))
        .clicked()
    {
        step(state, false);
    }
    let next = ui
        .add_enabled(count > 0, egui::Button::new("Nächster"))
        .clicked();
    if next || (entered && count > 0) {
        step(state, true);
    }
}

fn find_matches(query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    lowercase_lines()
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(needle.as_str()))
        .map(|(index, _)| index)
        .collect()
}

/// Moves the highlight to the next or previous match and scrolls to it.
fn step(state: &mut LicensesWindow, forward: bool) {
    let count = state.matches.len();
    if count == 0 {
        return;
    }
    let next = match (state.current, forward) {
        (None, true) => 0,
        (None, false) => count - 1,
        (Some(current), true) => (current + 1) % count,
        (Some(current), false) => (current + count - 1) % count,
    };
    state.current = Some(next);
    state.scroll_to = state.matches.get(next).copied();
}
