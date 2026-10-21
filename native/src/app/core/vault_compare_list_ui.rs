//! The body of the comparison window (U29-8): header with „Neu vergleichen“,
//! the filter of the groups, the collective actions, the list on the left
//! and the detail on the right.
use eframe::egui;

use super::vault_compare_detail_ui::{self as detail, DetailInput, DetailOut};
use super::vault_compare_ui::{start_action, start_compare, start_merge, Ask, Window};
use super::vault_models::VaultRow;
use super::vault_models_compare::{bulk_candidates, bulk_question, RowModel};
use super::vault_sidebar_ui::Effect;
use crate::daemon::CompareViewFull;
use crate::vault::{group_title, CompareGroup};

/// The labels of side A and side B.
fn labels(window: &Window, rows: &[VaultRow]) -> [String; 2] {
    let label = |index: usize| {
        window
            .view
            .as_ref()
            .map(|view| [&view.head.label_a, &view.head.label_b][index].clone())
            .filter(|label| !label.is_empty())
            .or_else(|| {
                rows.iter()
                    .find(|row| row.site == window.sites[index])
                    .map(|row| row.label.clone())
            })
            .unwrap_or_else(|| window.sites[index].clone())
    };
    [label(0), label(1)]
}

/// Draws the body of the window.
pub(super) fn content(
    ui: &mut egui::Ui,
    window: &mut Window,
    rows: &[VaultRow],
    effects: &mut Vec<Effect>,
) {
    let ctx = ui.ctx().clone();
    let names = labels(window, rows);
    let busy = window.busy();
    ui.horizontal(|ui| {
        ui.label(format!("Seite A: {} – Seite B: {}", names[0], names[1]));
        if ui
            .add_enabled(
                !busy && !window.locked,
                egui::Button::new("Neu vergleichen"),
            )
            .clicked()
        {
            start_compare(&ctx, window);
        }
        if busy {
            ui.spinner();
            ui.label(if window.comparing() {
                "Wird verglichen …"
            } else {
                "Aktion läuft …"
            });
        }
    });
    if let Some(error) = &window.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    if window.locked {
        for (index, site) in window.sites.clone().iter().enumerate() {
            if ui
                .button(format!("Entsperren … ({})", names[index]))
                .clicked()
            {
                effects.push(Effect::AskUnlock {
                    site: site.clone(),
                    then_open: false,
                });
            }
        }
    }
    filter_row(ui, window);
    let Some(view) = window.view.clone() else {
        return;
    };
    bulk_row(ui, window, &view, &names);
    ui.separator();
    let height = (ui.ctx().screen_rect().height() * 0.55).clamp(220.0, 560.0);
    let width = ui.available_width();
    ui.horizontal_top(|ui| {
        // Both columns stack their content: `allocate_ui` would take the
        // left-to-right layout of this row and lay the detail out sideways.
        let column = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(egui::vec2(width * 0.46, height), column, |ui| {
            list_ui(ui, window, &view, height);
        });
        ui.separator();
        ui.allocate_ui_with_layout(egui::vec2(width * 0.5, height), column, |ui| {
            detail_ui(ui, window, &view, rows, &names, effects);
        });
    });
}

fn filter_row(ui: &mut egui::Ui, window: &mut Window) {
    ui.horizontal_wrapped(|ui| {
        for group in CompareGroup::ALL {
            let count = window
                .view
                .as_ref()
                .and_then(|view| view.head.counts.get(group.key()).copied());
            let title = match count {
                Some(count) if group != CompareGroup::Structure || window.requested_structure => {
                    format!("{} ({count})", group_title(group))
                }
                _ => group_title(group).to_string(),
            };
            let mut on = window.filter.is_on(group);
            if ui.checkbox(&mut on, title).changed() {
                window.filter.set(group, on);
            }
        }
    });
    if window.filter.is_on(CompareGroup::Structure) != window.requested_structure {
        ui.small("Struktur: mit „Neu vergleichen“ laden.");
    }
}

fn bulk_row(ui: &mut egui::Ui, window: &mut Window, view: &CompareViewFull, names: &[String; 2]) {
    let count = bulk_candidates(&view.items);
    if count == 0 {
        return;
    }
    let busy = window.busy();
    ui.horizontal(|ui| {
        for (keep_a, title, label) in [
            (true, "Alle Konflikte: A", &names[0]),
            (false, "Alle Konflikte: B", &names[1]),
        ] {
            if ui.add_enabled(!busy, egui::Button::new(title)).clicked() {
                window.ask = Some(Ask::Bulk {
                    keep_a,
                    text: bulk_question(count, label),
                });
            }
        }
    });
}

fn list_ui(ui: &mut egui::Ui, window: &mut Window, view: &CompareViewFull, height: f32) {
    let list = window.rows.clone();
    if list.is_empty() {
        ui.label("Keine Unterschiede in den gewählten Gruppen.");
        return;
    }
    let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt("vault_compare_list")
        .max_height(height)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, list.len(), |ui, range| {
            for row in &list[range] {
                match row {
                    RowModel::Header { title, .. } => {
                        let size = [ui.available_width(), ui.spacing().interact_size.y];
                        ui.add_sized(size, egui::Label::new(egui::RichText::new(title).strong()));
                    }
                    RowModel::Item {
                        index,
                        label,
                        a,
                        b,
                        done,
                    } => {
                        let id = &view.items[*index].item;
                        let selected = window.selected.as_deref() == Some(id.as_str());
                        let mut text = format!("{label}    {a} · {b}");
                        if *done {
                            text.push_str("    ✓ erledigt");
                        }
                        let response = ui.selectable_label(selected, text);
                        if response.clicked() {
                            window.selected = Some(id.clone());
                        }
                        if let Some(error) = window.row_errors.get(id) {
                            response.on_hover_text(error.as_str());
                        }
                    }
                }
            }
        });
}

fn detail_ui(
    ui: &mut egui::Ui,
    window: &mut Window,
    view: &CompareViewFull,
    rows: &[VaultRow],
    names: &[String; 2],
    effects: &mut Vec<Effect>,
) {
    let Some(selected) = window.selected.clone() else {
        ui.label("Einen Eintrag links wählen.");
        return;
    };
    let Some(item) = view.items.iter().find(|item| item.item == selected) else {
        ui.label("Der Eintrag ist nicht mehr in der Liste.");
        return;
    };
    if let Some(error) = window.row_errors.get(&selected) {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    let input = DetailInput {
        job: &window.job,
        item,
        site_a: &view.head.site_a,
        site_b: &view.head.site_b,
        label_a: &names[0],
        label_b: &names[1],
        rows,
        busy: window.busy() || window.done.contains(&selected),
    };
    let out = detail::show(ui, &input, &mut window.detail);
    let ctx = ui.ctx().clone();
    match out {
        Some(DetailOut::Action { key, question }) => match question {
            Some(text) => {
                window.ask = Some(Ask::Item {
                    item: selected,
                    key,
                    text,
                });
            }
            None => start_action(&ctx, window, Some(selected), &key),
        },
        Some(DetailOut::Merge(text)) => start_merge(&ctx, window, selected, text),
        Some(DetailOut::Open { side_a }) => {
            let site = if side_a {
                &view.head.site_a
            } else {
                &view.head.site_b
            };
            effects.push(Effect::OpenVaultFile {
                site: site.clone(),
                path: item.path.clone(),
            });
        }
        Some(DetailOut::Unlock(site)) => effects.push(Effect::AskUnlock {
            site,
            then_open: false,
        }),
        None => {}
    }
}
