//! Area „Kodierung“ of the tab „Bildschirm“ (SCR-M3): the encoders and
//! decoders of this device, and OpenH264 by Cisco with Cisco's notice at its
//! switch and its license one click away („Lizenz anzeigen“, „Lizenzen“).
//! Since 2026-11-24 OpenH264 installs on its own (daemon start, a session
//! that needs H.264); the area shows „wird installiert“, „installiert“,
//! „abgeschaltet“ or „Fehler: <Grund> – neuer Versuch um …“, and on Windows
//! the state of the FFmpeg runtime the same way. „OpenH264 verwenden“
//! switches it off (no automatic installation fetches it then) and on again
//! (installed at once when missing); „Entfernen“ deletes the file and
//! switches it off. Inventory and actions run on a background thread; while
//! an installation is pending the area looks again every few seconds.
//! Plan 2026-11-17 „Windows-Bild“ P3: every row names its run-time check
//! (`CodecEntry::verdict`); `se screen codec verify` checks again.
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::screen::core::codec_registry::{
    CodecEntry, CodecReport, OpenH264State, OPENH264_NOTICE,
};
use crate::screen::os::codec_runtime_install::{
    ffmpeg_status, openh264_report, openh264_state_text, runtime_state_text, unix_now,
    RuntimeReport, RuntimeState,
};
use crate::screen::os::{codec_inventory, openh264_action, openh264_license, OpenH264Action};

/// How often the area looks again while an installation runs, and while one
/// is pending or failed.
const POLL_INSTALLING: Duration = Duration::from_secs(2);
const POLL_PENDING: Duration = Duration::from_secs(10);

#[derive(Default)]
struct CodecState {
    report: Option<CodecReport>,
    /// The FFmpeg runtime of the Windows viewer; `None` elsewhere.
    ffmpeg: Option<RuntimeReport>,
    loading: bool,
    /// A look at the installations runs (`poll`).
    polling: bool,
    polled_at: Option<Instant>,
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
            let ffmpeg = ffmpeg_status();
            let mut current = lock(&slot);
            current.report = Some(report);
            current.ffmpeg = ffmpeg;
            current.loading = false;
            current.polled_at = Some(Instant::now());
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).loading = false;
    }
}

/// How soon the area looks at the installations again (`openh264`: `None`
/// where OpenH264 is not offered); `None` while nothing is pending.
fn poll_interval(
    openh264: Option<OpenH264State>,
    ffmpeg: Option<RuntimeState>,
) -> Option<Duration> {
    let installing =
        openh264 == Some(OpenH264State::Installing) || ffmpeg == Some(RuntimeState::Installing);
    let pending = matches!(
        openh264,
        Some(OpenH264State::Absent | OpenH264State::Failed)
    ) || matches!(ffmpeg, Some(RuntimeState::Absent | RuntimeState::Failed));
    if installing {
        Some(POLL_INSTALLING)
    } else if pending {
        Some(POLL_PENDING)
    } else {
        None
    }
}

/// Looks at the OpenH264 and FFmpeg installations on a background thread;
/// a changed state reads the whole inventory again (the rows of the coders).
fn poll(shared: &Arc<Mutex<CodecState>>, ctx: &egui::Context) {
    lock(shared).polling = true;
    let (slot, ctx) = (shared.clone(), ctx.clone());
    let started = std::thread::Builder::new()
        .name("screen-codec-poll".into())
        .spawn(move || {
            let openh264 = openh264_report();
            let ffmpeg = ffmpeg_status();
            let known = {
                let current = lock(&slot);
                let openh264 = current.report.as_ref().map(|r| r.openh264.state);
                (openh264, current.ffmpeg.as_ref().map(|f| f.state))
            };
            let report = if known == (Some(openh264.state), ffmpeg.as_ref().map(|f| f.state)) {
                None
            } else {
                Some(codec_inventory::report())
            };
            let mut current = lock(&slot);
            if let Some(report) = report {
                current.report = Some(report);
            } else if let Some(shown) = current.report.as_mut() {
                shown.openh264 = openh264;
            }
            current.ffmpeg = ffmpeg;
            current.polling = false;
            current.polled_at = Some(Instant::now());
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).polling = false;
    }
}

/// Runs `action` on a background thread, then reads the inventory again.
fn run(shared: &Arc<Mutex<CodecState>>, ctx: &egui::Context, action: OpenH264Action) {
    let busy = match action {
        OpenH264Action::Enable { .. } => "Schalte OpenH264 ein und installiere es bei Bedarf …",
        OpenH264Action::Disable => "Schalte OpenH264 ab …",
        OpenH264Action::Remove => "Entferne OpenH264 und schalte es ab …",
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
            let ffmpeg = ffmpeg_status();
            let mut current = lock(&slot);
            current.error = result.err().map(|error| format!("OpenH264: {error}"));
            current.report = Some(report);
            current.ffmpeg = ffmpeg;
            current.busy = None;
            current.polled_at = Some(Instant::now());
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).busy = None;
    }
}

/// Checks every encoder again on a background thread (P3, as `se screen
/// codec verify`), then reads the register.
fn verify(shared: &Arc<Mutex<CodecState>>, ctx: &egui::Context) {
    {
        let mut current = lock(shared);
        current.busy = Some("Prüfe die Kodierer mit dem Testbild …");
        current.error = None;
    }
    let (slot, ctx) = (shared.clone(), ctx.clone());
    let started = std::thread::Builder::new()
        .name("screen-codec-verify".into())
        .spawn(move || {
            let (_, report) = codec_inventory::verify();
            let mut current = lock(&slot);
            current.report = Some(report);
            current.busy = None;
            ctx.request_repaint();
        });
    if started.is_err() {
        lock(shared).busy = None;
    }
}

/// The words of a run-time check (P3).
fn verdict_text(entry: &CodecEntry) -> String {
    let Some(verdict) = &entry.verdict else {
        return String::new();
    };
    let text = match (verdict.state.as_str(), verdict.reason.as_deref()) {
        ("verified", _) => "geprüft".to_string(),
        ("structural", _) => "Bitstrom geprüft".to_string(),
        ("failed", Some(reason)) => format!("Prüfung fehlgeschlagen ({reason}), nicht angeboten"),
        ("failed", None) => "Prüfung fehlgeschlagen, nicht angeboten".to_string(),
        _ => "ungeprüft".to_string(),
    };
    format!(" · {text}")
}

/// One line per codec: what, by whom, on what, and its check.
pub fn entry_line(entry: &CodecEntry) -> String {
    let profiles: Vec<&str> = entry.profiles.iter().map(|profile| profile.key()).collect();
    let kind = if entry.hardware {
        "Hardware"
    } else {
        "Software"
    };
    let verdict = verdict_text(entry);
    if profiles.is_empty() {
        format!("{} · {} · {kind}{verdict}", entry.codec.key(), entry.name)
    } else {
        format!(
            "{} ({}) · {} · {kind}{verdict}",
            entry.codec.key(),
            profiles.join(", "),
            entry.name
        )
    }
}

/// Draws the area (called from `share_screen_tab_ui.rs`).
pub fn ui(ui: &mut egui::Ui) {
    let shared = state().clone();
    let (report, ffmpeg, busy, error, license_open) = {
        let mut current = lock(&shared);
        if current.report.is_none() && !current.loading {
            current.loading = true;
            refresh(&shared, ui.ctx());
        }
        (
            current.report.clone(),
            current.ffmpeg.clone(),
            current.busy,
            current.error.clone(),
            current.license_open,
        )
    };
    theme::section(ui, "Bildschirmqualität und Kompatibilität");
    let Some(report) = report else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Lese die Kodierer …");
        });
        return;
    };
    ui.label(if report.encoders.is_empty() {
        "Bildschirmübertragung nicht verfügbar – kein geeigneter Kodierer vorhanden."
    } else {
        "Die Verbindung wählt automatisch eine gemeinsam unterstützte Bildübertragung."
    });
    egui::CollapsingHeader::new("Kodierungsdiagnose")
        .id_salt("screen_codec_diagnostics")
        .show(ui, |ui| {
            for (title, entries) in [
                ("Senden", &report.encoders),
                ("Empfangen", &report.decoders),
            ] {
                ui.strong(title);
                for entry in entries {
                    ui.label(RichText::new(entry_line(entry)).small());
                }
            }
            ui.small(format!("OpenH264-Version: {}", report.openh264.version));
            let button = egui::Button::new("Kodierer erneut prüfen");
            if ui.add_enabled(busy.is_none(), button).clicked() {
                verify(&shared, ui.ctx());
            }
        });
    let openh264 = &report.openh264;
    let now = unix_now();
    ui.horizontal_wrapped(|ui| {
        let mut wanted = openh264.state != OpenH264State::Disabled;
        let toggle = ui.add_enabled(
            busy.is_none() && openh264_license().is_some(),
            egui::Checkbox::new(&mut wanted, "OpenH264 verwenden"),
        );
        if toggle.changed() {
            match openh264_license() {
                Some((_, accepted)) if wanted => {
                    let accepted = accepted.to_string();
                    run(&shared, ui.ctx(), OpenH264Action::Enable { accepted });
                }
                _ => run(&shared, ui.ctx(), OpenH264Action::Disable),
            }
        }
        ui.label(OPENH264_NOTICE);
    });
    ui.horizontal_wrapped(|ui| {
        let text = format!(
            "Kompatibilitätskodierer: {}",
            openh264_state_text(openh264, now)
        );
        ui.label(RichText::new(text).small().color(theme::muted(ui)));
        if ui.button("Lizenz anzeigen").clicked() {
            lock(&shared).license_open = true;
        }
        if ui.button("Lizenzen").clicked() {
            lock(&shared).licenses_open = true;
        }
        let removable = openh264.library_present && busy.is_none();
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
    if let Some(ffmpeg) = &ffmpeg {
        let text = format!(
            "FFmpeg-Laufzeit {}: {}",
            ffmpeg.version,
            runtime_state_text(ffmpeg, now)
        );
        ui.label(RichText::new(text).small().color(theme::muted(ui)));
    }
    if let Some(error) = error {
        ui.colored_label(theme::danger(ui), error);
    }
    let offered = openh264_license().map(|_| openh264.state);
    if let Some(interval) = poll_interval(offered, ffmpeg.as_ref().map(|f| f.state)) {
        let current = lock(&shared);
        let due = current.polled_at.is_none_or(|at| at.elapsed() >= interval);
        if due && !current.polling && !current.loading && current.busy.is_none() {
            drop(current);
            poll(&shared, ui.ctx());
        }
        ui.ctx().request_repaint_after(interval);
    }
    if license_open {
        license_window(ui.ctx(), &shared);
    }
    if lock(&shared).licenses_open {
        licenses_window(ui.ctx(), &shared);
    }
}

/// Every embedded license text of the screen codecs, readable without a
/// download (`se screen codec licenses` shows the same).
fn licenses_window(ctx: &egui::Context, shared: &Arc<Mutex<CodecState>>) {
    let mut open = true;
    crate::app::dialog_geometry::window(ctx, "share_screen_codec_ui.0", "Bildschirm – Lizenzen")
        .id(egui::Id::new("Bildschirm – Lizenzen"))
        .open(&mut open)
        .default_size([680.0, 520.0])
        .max_size(theme::window_content_limit(ctx))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .max_height(crate::app::dialog_geometry::remaining_height(ui, 1.0))
                .show(ui, |ui| {
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

/// Cisco's license of OpenH264 in full, with Cisco's notice.
fn license_window(ctx: &egui::Context, shared: &Arc<Mutex<CodecState>>) {
    let Some((license, _)) = openh264_license() else {
        lock(shared).license_open = false;
        return;
    };
    let mut open = true;
    crate::app::dialog_geometry::window(
        ctx,
        "share_screen_codec_ui.1",
        "OpenH264 – Lizenz von Cisco",
    )
    .id(egui::Id::new("OpenH264 – Lizenz von Cisco"))
    .open(&mut open)
    .default_size([680.0, 520.0])
    .max_size(theme::window_content_limit(ctx))
    .show(ctx, |ui| {
        ui.label(OPENH264_NOTICE);
        egui::ScrollArea::vertical()
            .min_scrolled_height(0.0)
            .max_height(crate::app::dialog_geometry::remaining_height(ui, 1.0))
            .show(ui, |ui| {
                ui.label(RichText::new(license).monospace().small())
            });
    });
    if !open {
        lock(shared).license_open = false;
    }
}
