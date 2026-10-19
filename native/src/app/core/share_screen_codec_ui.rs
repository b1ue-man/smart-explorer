//! Area „Kodierung“ of the tab „Bildschirm“ (SCR-M3): the encoders and
//! decoders of this device, and OpenH264 by Cisco with the notice of
//! condition 3 at its switch. Switching on the first time opens the full
//! license text; only „Annehmen und herunterladen“ downloads from Cisco.
//! Off, on again and „Entfernen“ follow the switch. Inventory and actions
//! run on a background thread; the area shows their progress and errors.
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::screen::core::codec_registry::{
    CodecEntry, CodecReport, OpenH264State, OPENH264_NOTICE,
};
use crate::screen::os::{codec_inventory, openh264_action, openh264_license, OpenH264Action};

#[derive(Default)]
struct CodecState {
    report: Option<CodecReport>,
    loading: bool,
    busy: Option<&'static str>,
    error: Option<String>,
    license_open: bool,
    /// Bildschirm → Kodierung → Lizenzen (SCR-M10).
    licenses_open: bool,
}

fn state() -> &'static Arc<Mutex<CodecState>> {
    static STATE: OnceLock<Arc<Mutex<CodecState>>> = OnceLock::new();
    STATE.get_or_init(Arc::default)
}

fn lock(state: &Mutex<CodecState>) -> MutexGuard<'_, CodecState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads the inventory again on a background thread.
fn refresh(shared: &Arc<Mutex<CodecState>>, ctx: &egui::Context) {
    let (slot, ctx) = (shared.clone(), ctx.clone());
    let started = std::thread::Builder::new()
        .name("screen-codecs".into())
        .spawn(move || {
            let report = codec_inventory::report();
            let mut current = lock(&slot);
            current.report = Some(report);
            current.loading = false;
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).loading = false;
    }
}

/// Runs `action` on a background thread, then reads the inventory again.
fn run(shared: &Arc<Mutex<CodecState>>, ctx: &egui::Context, action: OpenH264Action) {
    let busy = match action {
        OpenH264Action::Enable { .. } => "Lade OpenH264 von Cisco und schalte es ein …",
        OpenH264Action::Disable => "Schalte OpenH264 aus …",
        OpenH264Action::Remove => "Entferne OpenH264 …",
    };
    {
        let mut current = lock(shared);
        current.busy = Some(busy);
        current.error = None;
    }
    let (slot, ctx) = (shared.clone(), ctx.clone());
    let started = std::thread::Builder::new()
        .name("screen-openh264".into())
        .spawn(move || {
            let result = openh264_action(&action);
            let report = codec_inventory::report();
            let mut current = lock(&slot);
            current.error = result.err().map(|error| format!("OpenH264: {error}"));
            current.report = Some(report);
            current.busy = None;
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).busy = None;
    }
}

/// One line per codec: what, by whom, on what.
pub(crate) fn entry_line(entry: &CodecEntry) -> String {
    let profiles: Vec<&str> = entry.profiles.iter().map(|profile| profile.key()).collect();
    let kind = if entry.hardware {
        "Hardware"
    } else {
        "Software"
    };
    if profiles.is_empty() {
        format!("{} · {} · {kind}", entry.codec.key(), entry.name)
    } else {
        format!(
            "{} ({}) · {} · {kind}",
            entry.codec.key(),
            profiles.join(", "),
            entry.name
        )
    }
}

fn state_text(state: OpenH264State) -> &'static str {
    match state {
        OpenH264State::Absent => "nicht heruntergeladen",
        OpenH264State::Enabled => "eingeschaltet",
        OpenH264State::Disabled => "ausgeschaltet",
        OpenH264State::Failed => "Datei passt nicht zur geprüften Fassung",
    }
}

/// Draws the area (called from `share_screen_tab_ui.rs`).
pub(crate) fn ui(ui: &mut egui::Ui) {
    let shared = state().clone();
    let (report, busy, error, license_open) = {
        let mut current = lock(&shared);
        if current.report.is_none() && !current.loading {
            current.loading = true;
            refresh(&shared, ui.ctx());
        }
        (
            current.report.clone(),
            current.busy,
            current.error.clone(),
            current.license_open,
        )
    };
    theme::section(ui, "Kodierung");
    let Some(report) = report else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Lese die Kodierer …");
        });
        return;
    };
    for (title, entries) in [
        ("Kodierer", &report.encoders),
        ("Dekodierer", &report.decoders),
    ] {
        ui.label(RichText::new(title).strong());
        for entry in entries {
            ui.label(RichText::new(entry_line(entry)).small());
        }
    }
    let openh264 = &report.openh264;
    ui.horizontal_wrapped(|ui| {
        let mut wanted = openh264.state == OpenH264State::Enabled;
        let toggle = ui.add_enabled(
            busy.is_none() && openh264_license().is_some(),
            egui::Checkbox::new(&mut wanted, "OpenH264 verwenden"),
        );
        if toggle.changed() {
            match (wanted, openh264.state) {
                (true, OpenH264State::Disabled) => {
                    if let Some((_, accepted)) = openh264_license() {
                        let accepted = accepted.to_string();
                        run(&shared, ui.ctx(), OpenH264Action::Enable { accepted });
                    }
                }
                (true, _) => lock(&shared).license_open = true,
                (false, _) => run(&shared, ui.ctx(), OpenH264Action::Disable),
            }
        }
        ui.label(OPENH264_NOTICE);
    });
    ui.horizontal_wrapped(|ui| {
        let reason = openh264
            .reason
            .as_deref()
            .map(|reason| format!(" ({reason})"));
        let text = format!(
            "OpenH264 {}: {}{}",
            openh264.version,
            state_text(openh264.state),
            reason.unwrap_or_default()
        );
        ui.label(RichText::new(text).small().color(theme::muted(ui)));
        if ui.button("Lizenz anzeigen").clicked() {
            lock(&shared).license_open = true;
        }
        if ui.button("Lizenzen").clicked() {
            lock(&shared).licenses_open = true;
        }
        let removable = openh264.state != OpenH264State::Absent && busy.is_none();
        if ui
            .add_enabled(removable, egui::Button::new("Entfernen"))
            .clicked()
        {
            run(&shared, ui.ctx(), OpenH264Action::Remove);
        }
        if let Some(busy) = busy {
            ui.spinner();
            ui.label(busy);
        }
    });
    if let Some(error) = error {
        ui.colored_label(theme::danger(ui), error);
    }
    if license_open {
        license_window(ui.ctx(), &shared, openh264.state, busy.is_none());
    }
    if lock(&shared).licenses_open {
        licenses_window(ui.ctx(), &shared);
    }
}

/// Every embedded license text of the screen codecs, readable without a
/// download (`se screen codec licenses` shows the same).
fn licenses_window(ctx: &egui::Context, shared: &Arc<Mutex<CodecState>>) {
    let mut open = true;
    egui::Window::new("Bildschirm – Lizenzen")
        .open(&mut open)
        .default_size([680.0, 520.0])
        .max_size(theme::window_content_limit(ctx))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in crate::screen::core::licenses::screen_licenses() {
                    ui.heading(entry.component);
                    ui.label(RichText::new(entry.license).strong());
                    ui.label(RichText::new(entry.text).monospace().small());
                    ui.separator();
                }
            });
        });
    if !open {
        lock(shared).licenses_open = false;
    }
}

/// The Cisco license in full, with the download as the only way to accept.
fn license_window(
    ctx: &egui::Context,
    shared: &Arc<Mutex<CodecState>>,
    state: OpenH264State,
    idle: bool,
) {
    let Some((license, accepted)) = openh264_license() else {
        lock(shared).license_open = false;
        return;
    };
    let mut open = true;
    let mut accept = false;
    egui::Window::new("OpenH264 – Lizenz von Cisco")
        .open(&mut open)
        .default_size([680.0, 520.0])
        .max_size(theme::window_content_limit(ctx))
        .show(ctx, |ui| {
            ui.label(OPENH264_NOTICE);
            egui::ScrollArea::vertical()
                .max_height(380.0)
                .show(ui, |ui| {
                    ui.label(RichText::new(license).monospace().small())
                });
            let downloadable = idle && state != OpenH264State::Enabled;
            let button = egui::Button::new("Annehmen und herunterladen");
            accept = ui.add_enabled(downloadable, button).clicked();
        });
    if accept {
        let accepted = accepted.to_string();
        run(shared, ctx, OpenH264Action::Enable { accepted });
        open = false;
    }
    if !open {
        lock(shared).license_open = false;
    }
}
