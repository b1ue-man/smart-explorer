//! The right half of the comparison window (U29-8): the metadata of both
//! versions, the line view of a text conflict, the merge view with a free
//! result field, the action buttons of the entry and „Öffnen (A)/(B)“.
//! There is no image comparison in the interface (the cleartext of the other
//! side would have to travel over the 2 MiB line of the service, a protocol
//! of its own, `docs/TODO.md` VAULT-COMPARE-IMAGE): everything that is not a
//! text offers the metadata and the open buttons.
use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_feed::{poll, spawn, Poll};
use super::vault_models::{failure_text, Failure, RowState, VaultRow};
use super::vault_models_compare::{
    buttons_for, item_label, merge_model, offers_text, open_sides, removed_root, row_mark,
    side_text, MergeModel,
};
use crate::daemon::CompareItemWire;
use crate::linemerge::Row;
use crate::vault::{group_title, CompareGroup};

type Texts = (String, String);

/// What the user asked for in the detail.
pub(super) enum DetailOut {
    /// An action button; `question` comes first when set.
    Action {
        key: String,
        question: Option<String>,
    },
    /// „Ergebnis speichern“ with the merged text.
    Merge(String),
    /// „Öffnen (A)“ / „Öffnen (B)“.
    Open { side_a: bool },
    /// „Entsperren …“ for a site.
    Unlock(String),
}

/// The state of the detail: the texts of the selected entry and the merge.
#[derive(Default)]
pub(super) struct DetailState {
    /// The entry the texts below are for.
    item: Option<String>,
    rx: Option<Receiver<Result<Texts, Failure>>>,
    merge: Option<MergeModel>,
    merging: bool,
    result: String,
    error: Option<String>,
}

impl DetailState {
    /// Drops everything with names or text of the vault.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// What the detail needs to know besides the entry.
pub(super) struct DetailInput<'a> {
    pub(super) job: &'a str,
    pub(super) item: &'a CompareItemWire,
    pub(super) site_a: &'a str,
    pub(super) site_b: &'a str,
    pub(super) label_a: &'a str,
    pub(super) label_b: &'a str,
    pub(super) rows: &'a [VaultRow],
    /// Actions are off (a lock, a running action).
    pub(super) busy: bool,
}

fn fetch_texts(ctx: &egui::Context, input: &DetailInput<'_>, state: &mut DetailState) {
    state.item = Some(input.item.item.clone());
    state.merge = None;
    state.merging = false;
    state.error = None;
    if !offers_text(input.item) {
        return;
    }
    let (job, item) = (input.job.to_string(), input.item.item.clone());
    match spawn(ctx, "vault-compare-text", move || {
        crate::daemon::vault_compare_text(&job, &item).map_err(|error| Failure {
            code: error.code,
            message: error.message,
        })
    }) {
        Ok(rx) => state.rx = Some(rx),
        Err(error) => state.error = Some(error),
    }
}

fn row_color(ui: &egui::Ui, row: &Row, left: bool) -> Option<egui::Color32> {
    if row.equal {
        return None;
    }
    let present = if left { &row.left } else { &row.right };
    let tint = if present.is_none() {
        crate::app::theme::muted(ui)
    } else if left {
        crate::app::theme::danger(ui)
    } else {
        crate::app::theme::success(ui)
    };
    Some(tint.gamma_multiply(0.25))
}

fn diff_view(ui: &mut egui::Ui, model: &MergeModel) {
    egui::ScrollArea::vertical()
        .min_scrolled_height(0.0)
        .id_salt("vault_compare_diff")
        .max_height(crate::app::dialog_geometry::remaining_height(ui, 2.0))
        .auto_shrink([false, true])
        .show_rows(ui, 18.0, model.rows.len(), |ui, range| {
            for index in range {
                let row = &model.rows[index];
                ui.horizontal(|ui| {
                    ui.monospace(row_mark(row));
                    for (left, text) in [(true, &row.left), (false, &row.right)] {
                        let shown = text.as_deref().unwrap_or("");
                        let mut label = egui::RichText::new(shown).monospace();
                        if let Some(color) = row_color(ui, row, left) {
                            label = label.background_color(color);
                        }
                        ui.add_sized(
                            [
                                (ui.available_width() / if left { 2.0 } else { 1.0 }).max(40.0),
                                18.0,
                            ],
                            egui::Label::new(label).truncate(),
                        );
                    }
                });
            }
        });
}

fn merge_view(ui: &mut egui::Ui, model: &mut MergeModel, result: &mut String) {
    ui.horizontal(|ui| {
        if ui.small_button("Alle A").clicked() {
            model.take_all(true);
            *result = model.result();
        }
        if ui.small_button("Alle B").clicked() {
            model.take_all(false);
            *result = model.result();
        }
    });
    let mut changed = false;
    let count = model.rows.len();
    egui::ScrollArea::vertical()
        .min_scrolled_height(0.0)
        .id_salt("vault_compare_merge")
        .max_height(crate::app::dialog_geometry::remaining_height(ui, 2.0))
        .auto_shrink([false, true])
        .show_rows(ui, 22.0, count, |ui, range| {
            for index in range {
                let row = &mut model.rows[index];
                let both = !row.equal && row.left.is_some() && row.right.is_some();
                ui.horizontal(|ui| {
                    if row.equal {
                        ui.monospace(row.left.as_deref().unwrap_or(""));
                        return;
                    }
                    if both {
                        if ui.selectable_label(row.take_left, "A").clicked() {
                            row.take_left = true;
                            row.take_right = false;
                            changed = true;
                        }
                        if ui.selectable_label(row.take_right, "B").clicked() {
                            row.take_right = true;
                            row.take_left = false;
                            changed = true;
                        }
                    } else if row.left.is_some() {
                        changed |= ui.checkbox(&mut row.take_left, "A").changed();
                    } else {
                        changed |= ui.checkbox(&mut row.take_right, "B").changed();
                    }
                    let text = row.left.as_deref().or(row.right.as_deref()).unwrap_or("");
                    let shown = if both {
                        format!(
                            "A: {} | B: {}",
                            row.left.as_deref().unwrap_or(""),
                            row.right.as_deref().unwrap_or("")
                        )
                    } else {
                        text.to_string()
                    };
                    ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).truncate());
                });
            }
        });
    if changed {
        *result = model.result();
    }
    ui.label("Ergebnis (frei bearbeitbar)");
    egui::ScrollArea::vertical()
        .min_scrolled_height(0.0)
        .id_salt("vault_compare_result")
        .max_height(crate::app::dialog_geometry::remaining_height(ui, 2.0))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(result)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
}

/// Draws the detail of one entry and returns what the user asked for.
pub(super) fn show(
    ui: &mut egui::Ui,
    input: &DetailInput<'_>,
    state: &mut DetailState,
) -> Option<DetailOut> {
    let ctx = ui.ctx().clone();
    if state.item.as_deref() != Some(input.item.item.as_str()) {
        fetch_texts(&ctx, input, state);
    }
    match poll(&mut state.rx) {
        Poll::Ready(Ok((a, b))) => match merge_model(&a, &b) {
            Ok(model) => {
                state.result = model.result();
                state.merge = Some(model);
            }
            Err(problem) => state.error = Some(problem),
        },
        Poll::Ready(Err(failure)) => state.error = Some(failure_text(&failure)),
        Poll::Gone => state.error = Some("Die Texte konnten nicht gelesen werden.".to_string()),
        Poll::Pending => {}
    }
    let item = input.item;
    let group = CompareGroup::from_key(&item.group);
    let mut out = None;
    ui.label(egui::RichText::new(item_label(item)).strong());
    if let Some(group) = group {
        ui.small(group_title(group));
    }
    if let Some(note) = &item.note {
        ui.small(note);
    }
    egui::Grid::new("vault_compare_meta")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(format!("Seite A ({})", input.label_a));
            ui.label(side_text(&item.a));
            ui.end_row();
            ui.label(format!("Seite B ({})", input.label_b));
            ui.label(side_text(&item.b));
            ui.end_row();
        });
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Texte werden gelesen …");
        });
    }
    if let Some(error) = &state.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    if state.merging {
        if let Some(model) = state.merge.as_mut() {
            merge_view(ui, model, &mut state.result);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!input.busy, egui::Button::new("Ergebnis speichern"))
                    .clicked()
                {
                    out = Some(DetailOut::Merge(state.result.clone()));
                }
                if ui.button("Zurück").clicked() {
                    state.merging = false;
                }
            });
            return out;
        }
    }
    if let Some(model) = &state.merge {
        diff_view(ui, model);
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        for button in buttons_for(item, &removed_root()) {
            if button.key == "merge" {
                let ready = state.merge.is_some();
                if ui
                    .add_enabled(ready && !input.busy, egui::Button::new(button.title))
                    .clicked()
                {
                    state.merging = true;
                }
                continue;
            }
            if ui
                .add_enabled(!input.busy, egui::Button::new(button.title))
                .clicked()
            {
                out = Some(DetailOut::Action {
                    key: button.key.clone(),
                    question: button.question.clone(),
                });
            }
        }
    });
    let sides = open_sides(item);
    for (index, (site, label, letter)) in [
        (input.site_a, input.label_a, "A"),
        (input.site_b, input.label_b, "B"),
    ]
    .into_iter()
    .enumerate()
    {
        if !sides[index] {
            continue;
        }
        let state_of = input
            .rows
            .iter()
            .find(|row| row.site == site)
            .map(|row| row.state);
        if state_of == Some(RowState::Unlocked) {
            if ui
                .add_enabled(!input.busy, egui::Button::new(format!("Öffnen ({letter})")))
                .clicked()
            {
                out = Some(DetailOut::Open { side_a: index == 0 });
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Zum Öffnen den Ort „{label}“ entsperren."));
                if ui.small_button("Entsperren …").clicked() {
                    out = Some(DetailOut::Unlock(site.to_string()));
                }
            });
        }
    }
    out
}
