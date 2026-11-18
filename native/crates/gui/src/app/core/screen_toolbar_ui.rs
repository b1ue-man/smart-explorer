//! The toolbar of the desktop viewer (SCR-M7, U25-7): device and source
//! choice, „Steuern“, fit and full screen, lowest latency (P9: the switch
//! „Schärfe vor Flüssigkeit“ is the codec choice „Scharf (GPU)“ now),
//! the keys egui cannot deliver (only those the host offers), the system
//! actions of an Android host, the statistics switch, and „Codec“,
//! „Dekodierer“ and „Höchste Datenrate“ (`screen_toolbar_codec.rs`, plan
//! 2026-11-14 „Bildkette“ 4.10). Apart from „Höchste Datenrate“, which it
//! stores itself, and „Niedrigste Latenz“, which it hands to the Direct3D 11
//! display at once (P9), it changes nothing: `show` returns the user's
//! choices, and the viewer app carries them out.
use eframe::egui::{self, RichText};

use super::screen_devices::DeviceEntry;
use super::screen_prefs::{Fit, ViewerPrefs};
use super::screen_session::SessionInfo;
use super::screen_target::ScreenTarget;
use super::screen_texts::blocked_text;
use crate::app::theme;
use crate::screen::core::app_icon::{IconRef, IconTarget};
use crate::screen::core::input_model::{
    SystemAction, KEY_CAPS_LOCK, KEY_MENU, KEY_MUTE, KEY_NUM_LOCK, KEY_PAUSE, KEY_PLAY_PAUSE,
    KEY_PRINT_SCREEN, KEY_SCROLL_LOCK, KEY_SUPER_LEFT, KEY_VOLUME_DOWN, KEY_VOLUME_UP,
};
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::ScreenHelloOk;
use crate::share::access_grant::CapabilityDecision;

/// The labels of the keys `TOOLBAR_KEYS` names.
pub const TOOLBAR_LABELS: [(u16, &str); 11] = [
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
pub fn toolbar_label(usage: u16) -> Option<&'static str> {
    TOOLBAR_LABELS
        .iter()
        .find(|(key, _)| *key == usage)
        .map(|(_, label)| *label)
}

/// The labels of the system actions of an Android host.
pub fn action_label(action: SystemAction) -> &'static str {
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
pub fn monitor_label(index: usize, name: &str, width: u32) -> String {
    if width == 0 {
        format!("Bildschirm {}: Auswahl am Gerät", index + 1)
    } else {
        format!("Bildschirm {}: {name}", index + 1)
    }
}

/// What the user chose in the toolbar.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarAction {
    Device(ScreenTarget),
    AllDevices,
    /// A source; `None` is the whole screen of a host that named no monitor.
    Source(Option<SourceId>),
    RefreshWindows,
    Control(bool),
    Fit(Fit),
    Fullscreen,
    /// „Codec“ (BK6, 4.10); `Lossless` keeps the old switch's meaning.
    Codec(super::screen_prefs::CodecSetting),
    /// „Dekodierer“ (BK6, 4.10).
    Decoder(super::screen_prefs::DecoderPreference),
    /// „Dekodierer-Bibliothek“ and „Anzeige“ (plan 2026-11-17 P8).
    DecoderLibrary(super::screen_prefs::DecoderLibrary),
    Display(super::screen_prefs::DisplayChoice),
    /// „Höchste Datenrate“ in bit/s, `None` automatic (B-09).
    MaxRate(Option<u64>),
    /// „Monitor“ and „Auflösung“ (plan 2026-11-17 P5).
    VirtualDisplay(super::screen_prefs::VirtualDisplayChoice),
    /// „Ton“: the host's sound on or off (plan 2026-11-17 P9).
    Audio(bool),
    LowLatency(bool),
    Stats(bool),
    Key(u16),
    System(SystemAction),
}

/// What the toolbar shows.
pub struct ToolbarInput<'a> {
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
pub fn input_possible(hello: &ScreenHelloOk) -> bool {
    let input = &hello.input;
    input.pointer || input.keyboard || input.text || input.touch || !input.actions.is_empty()
}

/// Whether „Steuern“ is offered: the right is not off and the host takes input.
pub fn control_offered(hello: &ScreenHelloOk) -> bool {
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
pub fn whole_screen(hello: &ScreenHelloOk) -> Option<SourceId> {
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
    if hello
        .limits
        .iter()
        .any(|limit| limit == "android_system_selected_source")
    {
        return "Android-Systemauswahl".into();
    }
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
    wrap::reserve_pair(ui, "Gerät:", input.current_name);
    wrap::item(ui, "device", |ui| device_list(ui, input, out));
    if wrap::item(ui, "all_devices", |ui| ui.button("Alle Geräte").clicked()) {
        out.push(ToolbarAction::AllDevices);
    }
}

fn device_list(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    ui.label("Gerät:");
    ({
        let text = input.current_name;
        wrap::fitted(
            ui,
            egui::ComboBox::from_id_salt("screen_viewer_device"),
            text,
        )
    })
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
}

fn source_icon(
    ui: &mut egui::Ui,
    input: &ToolbarInput,
    target: IconTarget,
    icon: Option<&IconRef>,
) {
    if let Some(info) = input.info {
        info.icons
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .show_target(ui, &info.icon_peer, target, icon, 22.0);
    }
}

fn source_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let Some(hello) = input.info.and_then(|info| info.hello.as_ref()) else {
        return;
    };
    let system_selected = hello
        .limits
        .iter()
        .any(|limit| limit == "android_system_selected_source");
    wrap::reserve_pair(ui, "Quelle:", &applied::short_text(&source_text(input)));
    let opened = wrap::item(ui, "source", |ui| {
        ui.label("Quelle:");
        ({
            let text = applied::short_text(&source_text(input));
            wrap::fitted(
                ui,
                egui::ComboBox::from_id_salt("screen_viewer_source"),
                text,
            )
        })
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(
                    input.source.is_none() || input.source == whole_screen(hello).as_ref(),
                    if system_selected {
                        "Android-Systemauswahl"
                    } else {
                        "Ganzer Bildschirm"
                    },
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
            if system_selected {
                ui.label("Die Quelle wird auf dem Android-Gerät im Systemdialog ausgewählt.");
            }
            if hello
                .limits
                .iter()
                .any(|limit| limit == "wayland_window_pick_on_host")
            {
                ui.label("Das Fenster wählt der Nutzer am Gerät im Systemdialog.");
            }
            let windows = input.info.and_then(|info| info.windows.as_ref());
            for window in windows
                .map(|list| list.windows.as_slice())
                .unwrap_or_default()
            {
                let source = SourceId::Window(window.id.clone());
                let label = format!("{} ({})", window.title, window.app);
                ui.push_id(("window", &window.id), |ui| {
                    ui.horizontal(|ui| {
                        source_icon(
                            ui,
                            input,
                            IconTarget::Window {
                                id: window.id.clone(),
                            },
                            window.icon.as_ref(),
                        );
                        let selectable = !hello
                            .limits
                            .iter()
                            .any(|limit| limit == "wayland_window_pick_on_host");
                        if ui
                            .add_enabled(
                                selectable,
                                egui::SelectableLabel::new(input.source == Some(&source), label),
                            )
                            .clicked()
                        {
                            out.push(ToolbarAction::Source(Some(source)));
                        }
                    })
                });
            }
            for app in windows.map(|list| list.apps.as_slice()).unwrap_or_default() {
                ui.push_id(("app", &app.app_id), |ui| {
                    ui.horizontal(|ui| {
                        source_icon(
                            ui,
                            input,
                            IconTarget::App {
                                app_id: app.app_id.clone(),
                            },
                            app.icon.as_ref(),
                        );
                        let source = SourceId::Window(app.app_id.clone());
                        let selectable = hello.platform
                            == crate::screen::core::wire_hello::HostPlatform::Android
                            && !system_selected
                            && SourceId::parse(&source.to_string()).is_ok();
                        if ui
                            .add_enabled(
                                selectable,
                                egui::SelectableLabel::new(
                                    input.source == Some(&source),
                                    &app.name,
                                ),
                            )
                            .clicked()
                        {
                            out.push(ToolbarAction::Source(Some(source)));
                        }
                    })
                });
            }
            if ui.button("Aktualisieren").clicked() {
                out.push(ToolbarAction::RefreshWindows);
            }
        })
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
    wrap::item(ui, "keys", |ui| {
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

#[cfg_attr(not(windows), allow(dead_code))] // library and display: Windows
#[path = "screen_toolbar_state.rs"]
mod applied;
#[path = "screen_toolbar_codec.rs"]
mod codec_rows;
pub use codec_rows::apply_codec;
#[path = "screen_toolbar_display.rs"]
mod display_rows;
#[path = "screen_toolbar_view.rs"]
mod view_rows;
#[path = "screen_toolbar_wrap.rs"]
mod wrap;
pub use display_rows::{apply_virtual_display, send_live};

/// Draws the toolbar and returns what the user chose. P9 (plan 2026-11-17):
/// every item keeps its whole width, so one that does not fit the rest of a
/// row moves to the next row as a whole; under egui's default a list or
/// label at the end of a row was squeezed into the space left (its text
/// wrapped inside the closed list) and items looked cut off.
pub fn show(ui: &mut egui::Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    let mut out = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        device_row(ui, input, &mut out);
        source_row(ui, input, &mut out);
        control_row(ui, input, &mut out);
        keys_menu(ui, input, &mut out);
        view_rows::view_row(ui, input, &mut out);
    });
    out
}
