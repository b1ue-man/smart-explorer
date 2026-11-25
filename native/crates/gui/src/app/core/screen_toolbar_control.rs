//! „Steuern“ and the menu „Tasten“ of the viewer's toolbar (moved out of
//! `screen_toolbar_ui.rs`, which stays below 500 lines): the switch only
//! where the host offers control, the keys and Android system actions only
//! that the host takes.
use eframe::egui::{self, RichText};

use super::{action_label, control_offered, toolbar_label, wrap, ToolbarAction, ToolbarInput};

pub(super) fn control_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    wrap::item(ui, "control", |ui| control_items(ui, input, out));
}

fn control_items(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let Some(hello) = input.info.and_then(|info| info.hello.as_ref()) else {
        return;
    };
    if !control_offered(hello) {
        return;
    }
    let mut on = input.control_on;
    let taken = input.controlled_by.is_some();
    // A disabled child never breaks its row itself (Gemini 3/8).
    wrap::reserve(ui, wrap::button_width(ui, "Steuern"));
    if ui
        .add_enabled(!taken, egui::SelectableLabel::new(on, "Steuern"))
        .on_hover_text("Strg+Umschalt+K")
        .clicked()
    {
        on = !on;
        out.push(ToolbarAction::Control(on));
    }
    if let Some(name) = input.controlled_by {
        wrap::note(ui, RichText::new(format!("Gesteuert von {name}")).small());
    }
    // A held or blocked input is named over the picture
    // (`ViewState::status_text`), not in the bar (Gemini 3/5.3: its line
    // changed the bar's height, the picture area and so the visibility).
}

pub(super) fn keys_menu(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let Some(hello) = input.info.and_then(|info| info.hello.as_ref()) else {
        return;
    };
    let keys: Vec<(u16, &str)> = hello
        .input
        .toolbar_keys
        .iter()
        .filter_map(|usage| toolbar_label(*usage).map(|label| (*usage, label)))
        .collect();
    let actions = &hello.input.actions;
    if keys.is_empty() && actions.is_empty() {
        return;
    }
    wrap::item(ui, "keys", |ui| {
        wrap::reserve(ui, wrap::button_width(ui, "Tasten ▾"));
        ui.add_enabled_ui(input.control_on, |ui| {
            ui.menu_button("Tasten ▾", |ui| {
                for (usage, label) in &keys {
                    if ui.button(*label).clicked() {
                        out.push(ToolbarAction::Key(*usage));
                        ui.close_menu();
                    }
                }
                if !keys.is_empty() && !actions.is_empty() {
                    ui.separator();
                }
                for action in actions {
                    if ui.button(action_label(*action)).clicked() {
                        out.push(ToolbarAction::System(*action));
                        ui.close_menu();
                    }
                }
            });
        });
    });
}
