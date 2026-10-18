//! Einstellungen → Integration → se Desktop → „Darstellung“ (DSK-M4, O4):
//! background (System, Farbe, Bild, Durchsichtig), the groups of the surface,
//! "Papierkorb" and "Dieser PC" (Windows), "Geräte öffnen mit" and the
//! desktop icons of other programs (Linux). A change goes through the step
//! of `se desktop config` (`Control::configure` with its rules) in a thread;
//! a running shell sends the new configuration to its surfaces within a
//! second (C5.5). What the choices depend on (alpha channel of the session,
//! mounted device tree) is read in a thread as well.
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::desktop::core::config::{
    Background, BackgroundKind, ConfigChange, ConfigContext, DesktopConfig, ForeignDesktopIcons,
    OpenDevicesWith,
};
use crate::desktop::core::state::Platform;
use crate::desktop::os::launch::with_control;
use crate::desktop::os::status::StatusReport;
use crate::desktop::os::wallpaper_image::{format_color, parse_color, DEFAULT_COLOR};
use crate::desktop::os::{feed_sources, platform, Session};

/// Image types the wallpaper decoder reads (H6).
const IMAGE_TYPES: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// What the choices depend on.
#[derive(Clone, Debug, Default)]
struct Facts {
    transparent: Option<Result<(), String>>,
    device_tree_mounted: bool,
}

#[derive(Default)]
struct Appearance {
    facts: Option<Facts>,
    loading: bool,
    busy: bool,
    message: Option<Result<String, String>>,
    /// The color being chosen, applied with "Übernehmen".
    color: Option<[u8; 3]>,
    /// The configuration of the last change until the status shows it.
    saved: Option<DesktopConfig>,
}

type Shared = Arc<Mutex<Appearance>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Appearance> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_facts(config: &DesktopConfig) -> Facts {
    Facts {
        transparent: Some(Session::probe(Some(config)).transparent),
        device_tree_mounted: feed_sources::device_tree_now().mounted(),
    }
}

fn load_facts(shared: &Shared, ctx: &egui::Context, config: &DesktopConfig) {
    {
        let mut panel = lock(shared);
        if panel.facts.is_some() || panel.loading {
            return;
        }
        panel.loading = true;
    }
    let (thread_shared, ctx, config) = (shared.clone(), ctx.clone(), config.clone());
    let spawned = std::thread::Builder::new()
        .name("se-desktop-facts".into())
        .spawn(move || {
            let facts = read_facts(&config);
            let mut panel = lock(&thread_shared);
            panel.facts = Some(facts);
            panel.loading = false;
            drop(panel);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        let mut panel = lock(shared);
        panel.facts = Some(Facts::default());
        panel.loading = false;
    }
}

fn apply(shared: &Shared, ctx: &egui::Context, change: ConfigChange) {
    {
        let mut panel = lock(shared);
        if panel.busy {
            return;
        }
        panel.busy = true;
        panel.message = None;
    }
    let (thread_shared, ctx) = (shared.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("se-desktop-config".into())
        .spawn(move || {
            let result = with_control(|control| {
                let stored = control.dir.read_config().ok().flatten();
                let context = ConfigContext {
                    platform: platform::PLATFORM,
                    transparent: Session::probe(stored.as_ref()).transparent,
                    device_tree_mounted: feed_sources::device_tree_now().mounted(),
                };
                control.configure(&change, &context)
            });
            let mut panel = lock(&thread_shared);
            panel.busy = false;
            panel.color = None;
            panel.facts = None;
            match result {
                Ok(config) => {
                    panel.saved = Some(config);
                    panel.message = Some(Ok("Darstellung gespeichert".into()));
                }
                Err(error) => panel.message = Some(Err(error.to_string())),
            }
            drop(panel);
            ctx.request_repaint();
        });
    if spawned.is_err() {
        let mut panel = lock(shared);
        panel.busy = false;
        panel.message = Some(Err("Hintergrundvorgang konnte nicht starten".into()));
    }
}

fn pick_image() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Bilder", &IMAGE_TYPES)
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

fn hint(ui: &mut egui::Ui, text: &str) {
    let color = theme::muted(ui);
    ui.label(RichText::new(text).small().color(color));
}

/// The section „Darstellung“ while se Desktop is added.
pub(in crate::app) fn ui_desktop_appearance(ui: &mut egui::Ui, report: &StatusReport) {
    let shared: Shared = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<Shared>(egui::Id::new("se_desktop_appearance"))
            .clone()
    });
    let (saved, facts, busy, message, color) = {
        let mut panel = lock(&shared);
        if panel.saved.is_some() && panel.saved == report.config {
            panel.saved = None;
        }
        (
            panel.saved.clone(),
            panel.facts.clone(),
            panel.busy,
            panel.message.clone(),
            panel.color,
        )
    };
    let Some(config) = saved.or_else(|| report.config.clone()) else {
        return;
    };
    load_facts(&shared, ui.ctx(), &config);
    let facts = facts.unwrap_or_default();
    ui.add_space(6.0);
    ui.label(RichText::new("Darstellung").strong());
    let mut change = ConfigChange::default();
    ui.add_enabled_ui(!busy, |ui| {
        background_ui(ui, &config, &facts, color, &shared, &mut change);
        groups_ui(ui, &config, report.platform, &mut change);
        open_with_ui(ui, &config, &facts, &mut change);
        if report.platform == Platform::Linux {
            icons_ui(ui, &config, &mut change);
        }
    });
    if change != ConfigChange::default() {
        apply(&shared, ui.ctx(), change);
    }
    match message {
        Some(Ok(text)) => {
            ui.colored_label(theme::success(ui), text);
        }
        Some(Err(text)) => {
            ui.colored_label(theme::danger(ui), text);
        }
        None => {}
    }
}

fn background_ui(
    ui: &mut egui::Ui,
    config: &DesktopConfig,
    facts: &Facts,
    color: Option<[u8; 3]>,
    shared: &Shared,
    change: &mut ConfigChange,
) {
    let kind = config.background.kind;
    let stored_color = config.background.value.as_deref().and_then(parse_color);
    let transparent_ok = matches!(facts.transparent, Some(Ok(())));
    let mut chosen = kind;
    ui.horizontal_wrapped(|ui| {
        ui.label("Hintergrund:");
        ui.radio_value(&mut chosen, BackgroundKind::System, "System");
        ui.radio_value(&mut chosen, BackgroundKind::Color, "Farbe");
        ui.radio_value(&mut chosen, BackgroundKind::Image, "Bild");
        ui.add_enabled_ui(transparent_ok, |ui| {
            ui.radio_value(&mut chosen, BackgroundKind::Transparent, "Durchsichtig");
        });
    });
    if chosen != kind {
        change.background = match chosen {
            BackgroundKind::System => Some(Background::system()),
            BackgroundKind::Color => Some(Background {
                kind: BackgroundKind::Color,
                value: Some(format_color(stored_color.unwrap_or(DEFAULT_COLOR))),
            }),
            BackgroundKind::Image => pick_image().map(|path| Background {
                kind: BackgroundKind::Image,
                value: Some(path),
            }),
            BackgroundKind::Transparent => Some(Background {
                kind: BackgroundKind::Transparent,
                value: None,
            }),
        };
    }
    if kind == BackgroundKind::Color {
        let mut rgb = color.or(stored_color).unwrap_or(DEFAULT_COLOR);
        ui.horizontal(|ui| {
            ui.label("Farbe:");
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                lock(shared).color = Some(rgb);
            }
            let pending = color.is_some_and(|picked| Some(picked) != stored_color);
            if ui
                .add_enabled(pending, egui::Button::new("Übernehmen"))
                .clicked()
            {
                change.background = Some(Background {
                    kind: BackgroundKind::Color,
                    value: Some(format_color(rgb)),
                });
            }
        });
    }
    if kind == BackgroundKind::Image {
        ui.horizontal_wrapped(|ui| {
            hint(ui, config.background.value.as_deref().unwrap_or_default());
            if ui.button("Anderes Bild …").clicked() {
                if let Some(path) = pick_image() {
                    change.background = Some(Background {
                        kind: BackgroundKind::Image,
                        value: Some(path),
                    });
                }
            }
        });
    }
    if let Some(Err(reason)) = &facts.transparent {
        hint(ui, &format!("Durchsichtig nicht möglich: {reason}"));
    }
}

fn flag(ui: &mut egui::Ui, label: &str, current: bool, change: &mut Option<bool>) {
    let mut on = current;
    if ui.checkbox(&mut on, label).changed() {
        *change = Some(on);
    }
}

fn groups_ui(ui: &mut egui::Ui, config: &DesktopConfig, on: Platform, change: &mut ConfigChange) {
    let groups = config.groups;
    ui.horizontal_wrapped(|ui| {
        ui.label("Zeigen:");
        flag(
            ui,
            "Desktop-Dateien",
            groups.desktop_files,
            &mut change.desktop_files,
        );
        flag(ui, "Geräte", groups.devices, &mut change.devices);
        flag(ui, "Räume", groups.rooms, &mut change.rooms);
        flag(ui, "Tresore", groups.vaults, &mut change.vaults);
        flag(ui, "Alle Bildschirme", groups.screens, &mut change.screens);
        if on == Platform::Windows {
            let items = config.system_items;
            flag(ui, "Papierkorb", items.recycle_bin, &mut change.recycle_bin);
            flag(ui, "Dieser PC", items.this_pc, &mut change.this_pc);
        }
    });
}

fn open_with_ui(
    ui: &mut egui::Ui,
    config: &DesktopConfig,
    facts: &Facts,
    change: &mut ConfigChange,
) {
    let mounted = facts.device_tree_mounted;
    let mut chosen = config.open_devices_with;
    ui.horizontal_wrapped(|ui| {
        ui.label("Geräte öffnen mit:");
        ui.radio_value(&mut chosen, OpenDevicesWith::App, "Smart Explorer");
        ui.add_enabled_ui(mounted, |ui| {
            ui.radio_value(&mut chosen, OpenDevicesWith::FileManager, "Dateimanager");
        });
    });
    if chosen != config.open_devices_with {
        change.open_devices_with = Some(chosen);
    }
    if !mounted {
        hint(
            ui,
            "Dateimanager: erst mit eingehängten „Verbundenen Geräten als Ordner“ (Abschnitt \
             oben); bis dahin öffnet Smart Explorer die Geräte.",
        );
    }
}

fn icons_ui(ui: &mut egui::Ui, config: &DesktopConfig, change: &mut ConfigChange) {
    let mut chosen = config.foreign_desktop_icons;
    ui.horizontal_wrapped(|ui| {
        ui.label("Desktopsymbole anderer Programme:");
        ui.radio_value(&mut chosen, ForeignDesktopIcons::Keep, "behalten");
        ui.radio_value(&mut chosen, ForeignDesktopIcons::TakeOver, "übernehmen");
    });
    if chosen != config.foreign_desktop_icons {
        change.foreign_desktop_icons = Some(chosen);
    }
    hint(
        ui,
        "„Übernehmen“ blendet die Desktopsymbole der GNOME-Erweiterung aus, solange se \
         Desktop läuft; Deaktivieren und Entfernen geben sie zurück.",
    );
}
