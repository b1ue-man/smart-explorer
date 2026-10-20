//! The toolbar of the desktop viewer (SCR-M7, U25-7): device and source
//! choice, „Steuern“, fit and full screen, sharpness, lossless and lowest
//! latency, the keys egui cannot deliver (only those the host offers), the
//! system actions of an Android host and the statistics switch. It changes
//! nothing itself: `show` returns the user's choices, and the viewer app
//! carries them out.
use eframe::egui::{self, RichText};

use super::screen_devices::DeviceEntry;
use super::screen_prefs::{Fit, ViewerPrefs};
use super::screen_session::SessionInfo;
use super::screen_target::ScreenTarget;
use super::screen_texts::blocked_text;
use crate::app::theme;
use crate::screen::core::input_model::{
    SystemAction, KEY_CAPS_LOCK, KEY_MENU, KEY_MUTE, KEY_NUM_LOCK, KEY_PAUSE, KEY_PLAY_PAUSE,
    KEY_PRINT_SCREEN, KEY_SCROLL_LOCK, KEY_SUPER_LEFT, KEY_VOLUME_DOWN, KEY_VOLUME_UP,
};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::ScreenHelloOk;
use crate::share::access_grant::CapabilityDecision;

/// The labels of the keys `TOOLBAR_KEYS` names.
pub(crate) const TOOLBAR_LABELS: [(u16, &str); 11] = [
    (KEY_SUPER_LEFT, "Windows-Taste"),
    (KEY_MENU, "Menütaste"),
    (KEY_PRINT_SCREEN, "Druck"),
    (KEY_PAUSE, "Pause"),
    (KEY_SCROLL_LOCK, "Rollen"),
    (KEY_CAPS_LOCK, "Feststelltaste"),
    (KEY_NUM_LOCK, "NumLock"),
    (KEY_PLAY_PAUSE, "Wiedergabe/Pause"),
    (KEY_VOLUME_UP, "Lauter"),
    (KEY_VOLUME_DOWN, "Leiser"),
    (KEY_MUTE, "Stumm"),
];

/// The label of a toolbar key.
pub(crate) fn toolbar_label(usage: u16) -> Option<&'static str> {
    TOOLBAR_LABELS
        .iter()
        .find(|(key, _)| *key == usage)
        .map(|(_, label)| *label)
}

/// The labels of the system actions of an Android host.
pub(crate) fn action_label(action: SystemAction) -> &'static str {
    match action {
        SystemAction::Back => "Zurück",
        SystemAction::Home => "Start",
        SystemAction::Recents => "Übersicht",
        SystemAction::Notifications => "Benachrichtigungen",
        SystemAction::QuickSettings => "Schnelleinstellungen",
        SystemAction::LockScreen => "Sperren",
        SystemAction::PowerDialog => "Ausschalten-Dialog",
    }
}

/// The text a monitor gets in the source choice.
pub(crate) fn monitor_label(index: usize, name: &str, width: u32) -> String {
    if width == 0 {
        format!("Bildschirm {}: Auswahl am Gerät", index + 1)
    } else {
        format!("Bildschirm {}: {name}", index + 1)
    }
}

/// What the user chose in the toolbar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ToolbarAction {
    Device(ScreenTarget),
    AllDevices,
    /// A source; `None` is the whole screen of a host that named no monitor.
    Source(Option<SourceId>),
    RefreshWindows,
    Control(bool),
    Fit(Fit),
    Fullscreen,
    Sharpness(bool),
    Lossless(bool),
    LowLatency(bool),
    Stats(bool),
    Key(u16),
    System(SystemAction),
}

/// What the toolbar shows.
pub(crate) struct ToolbarInput<'a> {
    pub devices: &'a [DeviceEntry],
    pub current: &'a ScreenTarget,
    pub current_name: &'a str,
    pub info: Option<&'a SessionInfo>,
    pub source: Option<&'a SourceId>,
    pub prefs: &'a ViewerPrefs,
    pub control_on: bool,
    /// Another device holds the control of the host.
    pub controlled_by: Option<&'a str>,
    pub fullscreen: bool,
    pub stats_on: bool,
    /// „Niedrigste Latenz“ was changed since the start.
    pub low_latency_changed: bool,
}

/// Whether the host can take any input at all.
pub(crate) fn input_possible(hello: &ScreenHelloOk) -> bool {
    let input = &hello.input;
    input.pointer || input.keyboard || input.text || input.touch || !input.actions.is_empty()
}

/// Whether „Steuern“ is offered: the right is not off and the host takes input.
pub(crate) fn control_offered(hello: &ScreenHelloOk) -> bool {
    hello.authorization.control != CapabilityDecision::Off && input_possible(hello)
}

/// The source choices of a host: the whole screen, each monitor.
fn monitor_sources(hello: &ScreenHelloOk) -> Vec<(String, SourceId)> {
    hello
        .monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            (
                monitor_label(index, &monitor.name, monitor.width),
                SourceId::Monitor(monitor.id.clone()),
            )
        })
        .collect()
}

/// The source „Ganzer Bildschirm“ stands for: the primary monitor.
pub(crate) fn whole_screen(hello: &ScreenHelloOk) -> Option<SourceId> {
    hello
        .monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| hello.monitors.first())
        .map(|monitor| SourceId::Monitor(monitor.id.clone()))
}

fn source_text(input: &ToolbarInput) -> String {
    let (Some(hello), Some(source)) = (
        input.info.and_then(|info| info.hello.as_ref()),
        input.source,
    ) else {
        return "Ganzer Bildschirm".into();
    };
    if whole_screen(hello).as_ref() == Some(source) {
        return "Ganzer Bildschirm".into();
    }
    match source {
        SourceId::Monitor(id) => hello
            .monitors
            .iter()
            .position(|monitor| monitor.id == *id)
            .map_or_else(
                || "Bildschirm".to_string(),
                |index| {
                    monitor_label(
                        index,
                        &hello.monitors[index].name,
                        hello.monitors[index].width,
                    )
                },
            ),
        SourceId::Window(id) => input
            .info
            .and_then(|info| info.windows.as_ref())
            .and_then(|list| list.windows.iter().find(|window| window.id == *id))
            .map_or_else(
                || "Fenster".to_string(),
                |window| format!("{} ({})", window.title, window.app),
            ),
        SourceId::Synthetic => "Testbild".into(),
    }
}

fn device_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    ui.label("Gerät:");
    egui::ComboBox::from_id_salt("screen_viewer_device")
        .selected_text(input.current_name)
        .show_ui(ui, |ui| {
            for entry in input.devices {
                let label = match &entry.room {
                    Some(room) => format!("{} ({room})", entry.name),
                    None => entry.name.clone(),
                };
                let selected = entry.target.same_device(input.current);
                if ui.selectable_label(selected, label).clicked() && !selected {
                    out.push(ToolbarAction::Device(entry.target.clone()));
                }
            }
        });
    if ui.button("Alle Geräte").clicked() {
        out.push(ToolbarAction::AllDevices);
    }
}

fn source_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let Some(hello) = input.info.and_then(|info| info.hello.as_ref()) else {
        return;
    };
    ui.label("Quelle:");
    let opened = egui::ComboBox::from_id_salt("screen_viewer_source")
        .selected_text(source_text(input))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(
                    input.source.is_none() || input.source == whole_screen(hello).as_ref(),
                    "Ganzer Bildschirm",
                )
                .clicked()
            {
                out.push(ToolbarAction::Source(whole_screen(hello)));
            }
            for (label, source) in monitor_sources(hello) {
                if ui
                    .selectable_label(input.source == Some(&source), label)
                    .clicked()
                {
                    out.push(ToolbarAction::Source(Some(source)));
                }
            }
            ui.separator();
            if hello
                .limits
                .iter()
                .any(|limit| limit == "wayland_window_pick_on_host")
            {
                ui.label("Das Fenster wählt der Nutzer am Gerät im Systemdialog.");
                return;
            }
            let windows = input.info.and_then(|info| info.windows.as_ref());
            for window in windows
                .map(|list| list.windows.as_slice())
                .unwrap_or_default()
            {
                let source = SourceId::Window(window.id.clone());
                let label = format!("{} ({})", window.title, window.app);
                if ui
                    .selectable_label(input.source == Some(&source), label)
                    .clicked()
                {
                    out.push(ToolbarAction::Source(Some(source)));
                }
            }
            if ui.button("Aktualisieren").clicked() {
                out.push(ToolbarAction::RefreshWindows);
            }
        });
    // The list is read once when the field opens.
    let id = ui.id().with("screen_viewer_windows_asked");
    let asked = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    match (opened.inner.is_some(), asked) {
        (true, false) => {
            out.push(ToolbarAction::RefreshWindows);
            ui.data_mut(|data| data.insert_temp(id, true));
        }
        (false, true) => ui.data_mut(|data| data.insert_temp(id, false)),
        _ => {}
    }
}

fn control_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let Some(hello) = input.info.and_then(|info| info.hello.as_ref()) else {
        return;
    };
    if !control_offered(hello) {
        return;
    }
    let mut on = input.control_on;
    let taken = input.controlled_by.is_some();
    if ui
        .add_enabled(!taken, egui::SelectableLabel::new(on, "Steuern"))
        .on_hover_text("Strg+Umschalt+K")
        .clicked()
    {
        on = !on;
        out.push(ToolbarAction::Control(on));
    }
    if let Some(name) = input.controlled_by {
        ui.label(RichText::new(format!("Gesteuert von {name}")).small());
    }
    let blocked: Option<ControlBlock> = input.info.and_then(|info| info.blocked);
    if let (Some(block), true) = (blocked, input.control_on) {
        ui.colored_label(theme::warning(ui), blocked_text(block));
    }
}

fn keys_menu(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
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
}

fn view_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let fit = input.prefs.fit;
    if ui.selectable_label(fit == Fit::Fit, "Einpassen").clicked() && fit != Fit::Fit {
        out.push(ToolbarAction::Fit(Fit::Fit));
    }
    if ui.selectable_label(fit == Fit::Actual, "1:1").clicked() && fit != Fit::Actual {
        out.push(ToolbarAction::Fit(Fit::Actual));
    }
    if ui
        .selectable_label(input.fullscreen, "Vollbild")
        .on_hover_text("F11")
        .clicked()
    {
        out.push(ToolbarAction::Fullscreen);
    }
    let mut toggle =
        |ui: &mut egui::Ui, value: bool, text: &str, make: fn(bool) -> ToolbarAction| {
            let mut changed = value;
            if ui.checkbox(&mut changed, text).changed() {
                out.push(make(changed));
            }
        };
    toggle(
        ui,
        input.prefs.sharpness,
        "Schärfe vor Flüssigkeit",
        ToolbarAction::Sharpness,
    );
    toggle(
        ui,
        input.prefs.lossless,
        "Verlustfrei",
        ToolbarAction::Lossless,
    );
    toggle(
        ui,
        input.prefs.low_latency,
        "Niedrigste Latenz (Bildrisse möglich)",
        ToolbarAction::LowLatency,
    );
    if input.low_latency_changed {
        ui.label(RichText::new("Wirkt beim nächsten Start des Betrachters.").small());
    }
    toggle(ui, input.stats_on, "Statistik", ToolbarAction::Stats);
}

/// Draws the toolbar and returns what the user chose.
pub(crate) fn show(ui: &mut egui::Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    let mut out = Vec::new();
    ui.horizontal_wrapped(|ui| {
        device_row(ui, input, &mut out);
        source_row(ui, input, &mut out);
        control_row(ui, input, &mut out);
        keys_menu(ui, input, &mut out);
        view_row(ui, input, &mut out);
    });
    out
}
