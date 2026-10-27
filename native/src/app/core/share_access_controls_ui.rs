//! The two cards of the access dialog: „Rechte“ (every right with Aus,
//! Fragen, Erlauben, and the period) and „Befehle ausführen“ (preset,
//! account, confirmation, program list). 2026-10-27: no single-entry lists
//! and no heading per single control.
use super::*;
use crate::share::access_grant::AccessPreset::{FullOwn, Programs, RemoteAccount};

/// Card „Rechte“ (one row per right, Aus/Fragen/Erlauben, and the period)
/// and card „Befehle ausführen“ (preset, account, confirmation, programs).
pub(in crate::app) fn controls(
    ui: &mut egui::Ui,
    draft: &mut AccessDraft,
    own_label: &str,
    other_accounts: bool,
) {
    use crate::app::device_page_layout::card;
    card(ui, "Rechte", None, |ui| {
        rights_table(ui, draft);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Gültig");
            period_choice(ui, draft);
        });
    });
    card(
        ui,
        "Befehle ausführen",
        Some("Gilt für Befehle und Terminal"),
        |ui| run_as(ui, draft, own_label, other_accounts),
    );
}

fn rights_table(ui: &mut egui::Ui, draft: &mut AccessDraft) {
    let entries = catalog(draft.helper);
    let wide = ui.available_width() >= egui::TextStyle::Body.resolve(ui.style()).size * 34.0;
    if !wide {
        for entry in &entries {
            capability_row(ui, draft, entry);
        }
        return;
    }
    egui::Grid::new(("capability_table", &draft.key))
        .num_columns(4)
        .striped(true)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            ui.label("");
            let headers = [
                ui.strong("Aus").id,
                ui.strong("Fragen").id,
                ui.strong("Erlauben").id,
            ];
            ui.end_row();
            for entry in &entries {
                let label = ui.label(entry.label).on_hover_text(entry.description);
                for (column, decision) in [Off, Ask, Allow].into_iter().enumerate() {
                    capability_option(ui, draft, entry, decision, "")
                        .labelled_by(label.id)
                        .labelled_by(headers[column]);
                }
                ui.end_row();
            }
        });
}

fn run_as(ui: &mut egui::Ui, draft: &mut AccessDraft, own_label: &str, other_accounts: bool) {
    let remote = draft
        .accounts
        .iter()
        .find(|o| o.profile == AccountProfile::Remote)
        .cloned();
    let presets = [
        (
            FullOwn,
            format!("Volle Rechte als mein Konto ({own_label})"),
            true,
        ),
        (
            RemoteAccount,
            "Eingeschränktes Fernzugriffskonto se-remote".into(),
            remote.as_ref().is_some_and(|o| o.available),
        ),
        (Programs, "Nur bestimmte Programme".into(), true),
    ];
    for (preset, label, enabled) in presets {
        if preset == RemoteAccount && remote.is_none() {
            continue;
        }
        let response = ui.add_enabled(
            enabled,
            egui::RadioButton::new(draft.preset == Some(preset), label),
        );
        if response.clicked() {
            draft.select(preset);
        }
    }
    if let Some(reason) = remote.and_then(|option| option.reason) {
        ui.small(reason);
    }
    if draft.preset.is_none() {
        ui.label(
            RichText::new("Eigene Auswahl aus den Rechten oben")
                .small()
                .color(theme::muted(ui)),
        );
    }
    // A list only when there is something to choose.
    let choices = draft
        .accounts
        .iter()
        .filter(|option| option.available)
        .count();
    ui.horizontal_wrapped(|ui| {
        ui.label("Ausführen als");
        if choices > 1 {
            egui::ComboBox::from_id_salt(("share_access_account", &draft.key))
                .selected_text(account_name(&draft.account, own_label))
                .show_ui(ui, |ui| {
                    for option in draft.accounts.clone() {
                        let label = account_name(&option.profile, own_label);
                        let chosen = option.profile == draft.account;
                        let response = ui.add_enabled(
                            option.available,
                            egui::SelectableLabel::new(chosen, label),
                        );
                        if response.clicked() {
                            draft.set_account(option.profile);
                        }
                    }
                });
        } else {
            ui.strong(account_name(&draft.account, own_label));
        }
    });
    if other_accounts {
        ui.horizontal_wrapped(|ui| {
            ui.label("Anderes lokales Konto");
            ui.add(egui::TextEdit::singleline(&mut draft.account_input).desired_width(140.0));
            if ui.button("Übernehmen").clicked() {
                let account = AccountProfile::Account {
                    name: draft.account_input.trim().into(),
                };
                draft.account_error = account.validate().err();
                if draft.account_error.is_none() {
                    draft.set_account(account);
                }
            }
        });
        if let Some(error) = &draft.account_error {
            ui.colored_label(theme::danger(ui), error);
        }
    }
    let mut ask = draft.ask;
    if ui
        .checkbox(&mut ask, "Jeden Start an diesem Gerät bestätigen lassen")
        .changed()
    {
        draft.set_ask(ask);
    }
    let title = format!("Programmliste ({})", draft.programs.len());
    egui::CollapsingHeader::new(title)
        .id_salt(("share_access_programs", &draft.key))
        .default_open(draft.preset == Some(Programs))
        .show(ui, |ui| {
            let (mut list, mut input, mut error) = (
                draft.programs.clone(),
                std::mem::take(&mut draft.program_input),
                draft.error.take(),
            );
            if list_editor(ui, &draft.key, &mut list, &mut input, &mut error) {
                draft.set_programs(list);
            }
            (draft.program_input, draft.error) = (input, error);
        });
}
