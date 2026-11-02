//! „Mehr“ in the rights window (plan 2026-10-31 §F, F3): the account commands
//! run as, and the shortcuts to the settings that are not part of a preset –
//! program list, system locations, TCP rules and share-back.
use eframe::egui::{self, RichText};

use super::share_rights_model::{RightsDraft, RightsSubject};
use crate::app::theme;
use crate::share::access_grant::AccountProfile;

/// What the window knows about the device for this section.
pub(super) struct MoreInfo {
    /// The pinned device (Direct grant) whose page holds program list and
    /// TCP rules; `None` for a room or the Direct code.
    pub(super) device: Option<(String, String)>,
    /// Entries of the grant's program list.
    pub(super) programs: usize,
    /// The saved contact of the device and its „Rückfreigabe“ switch.
    pub(super) contact: Option<(String, bool)>,
    /// Another account than the worker's can be chosen on this host.
    pub(super) other_accounts: bool,
}

/// A click that needs the app, handled by the window.
pub(super) enum MoreAction {
    DevicePage(String, String),
    Exports,
    ShareBack(String, bool),
}

pub(super) fn ui(
    ui: &mut egui::Ui,
    draft: &mut RightsDraft,
    info: &MoreInfo,
) -> Option<MoreAction> {
    let mut action = None;
    egui::Grid::new("share_rights_more_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Konto");
            account(ui, draft, info.other_accounts);
            ui.end_row();
            if let Some((device, key)) = &info.device {
                ui.label("Programmliste");
                ui.horizontal_wrapped(|ui| {
                    let text = match info.programs {
                        0 => "Keine Beschränkung (volle Shell)".to_string(),
                        count => format!("Nur {count} erlaubte Programme, kein Terminal"),
                    };
                    ui.label(text);
                    if ui.button("Bearbeiten …").clicked() {
                        action = Some(MoreAction::DevicePage(device.clone(), key.clone()));
                    }
                });
                ui.end_row();
                ui.label("TCP-Regeln");
                if ui
                    .button("Dienste und TCP …")
                    .on_hover_text("Öffnet die Geräteseite; Desktop steuern schließt lokale Dienste ein, einzelne Regeln stehen dort.")
                    .clicked()
                {
                    action = Some(MoreAction::DevicePage(device.clone(), key.clone()));
                }
                ui.end_row();
            }
            ui.label("Systemorte");
            if ui
                .button("Freigaben …")
                .on_hover_text("Schreiben in Systemorte ist je Ordner in den Freigaben einstellbar.")
                .clicked()
            {
                action = Some(MoreAction::Exports);
            }
            ui.end_row();
            if let Some((contact, share_back)) = &info.contact {
                ui.label("Rückfreigabe");
                let mut on = *share_back;
                if ui
                    .checkbox(&mut on, "Meine Freigaben für dieses Gerät offen halten")
                    .changed()
                {
                    action = Some(MoreAction::ShareBack(contact.clone(), on));
                }
                ui.end_row();
            }
        });
    if matches!(draft.subject, RightsSubject::Code) {
        ui.label(
            RichText::new("Der Code vergibt diese Rechte an jedes Gerät, das mit ihm verbindet.")
                .small()
                .color(theme::muted(ui)),
        );
    }
    action
}

fn account(ui: &mut egui::Ui, draft: &mut RightsDraft, other_accounts: bool) {
    ui.vertical(|ui| {
        let current = draft.rights.account.clone();
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(current == AccountProfile::Own, "Mein Konto")
                .on_hover_text("Befehle und Terminal laufen als der Benutzer von Smart Explorer.")
                .clicked()
            {
                draft.set_account(AccountProfile::Own);
            }
            if other_accounts {
                if ui
                    .selectable_label(current == AccountProfile::Remote, "Eingeschränktes Konto")
                    .on_hover_text("Das Fernzugriffskonto se-remote.")
                    .clicked()
                {
                    draft.set_account(AccountProfile::Remote);
                }
                let named = matches!(current, AccountProfile::Account { .. });
                if ui.selectable_label(named, "Anderes Konto").clicked() && !named {
                    draft.set_account(AccountProfile::Account {
                        name: String::new(),
                    });
                }
            }
        });
        if let AccountProfile::Account { name } = &current {
            let mut edited = name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut edited).hint_text("Kontoname"))
                .changed()
            {
                draft.set_account(AccountProfile::Account { name: edited });
            }
        }
        if !other_accounts {
            ui.label(
                RichText::new("Dieses Gerät führt Befehle nur als sein eigener Benutzer aus.")
                    .small()
                    .color(theme::muted(ui)),
            );
        }
    });
}
