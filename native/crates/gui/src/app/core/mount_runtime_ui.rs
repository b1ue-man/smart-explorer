use std::time::Instant;

use eframe::egui;

use super::App;

#[cfg(not(target_os = "linux"))]
const DOKANY_RELEASE_URL: &str = "https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000";

/// Windows: the button that installs the pinned Dokany runtime; `true` when
/// it was clicked.
#[cfg(not(target_os = "linux"))]
pub(super) fn install_controls(ui: &mut egui::Ui, enabled: bool, link_label: &str) -> bool {
    let clicked = ui
        .add_enabled(enabled, egui::Button::new("Dokany 2.3.1 installieren"))
        .on_hover_text(
            "Laedt ausschliesslich die fest gepinnte offizielle x64-MSI und prueft Groesse, SHA-256 sowie Windows-Signatur vor der Administratorabfrage.",
        )
        .clicked();
    ui.hyperlink_to(link_label, DOKANY_RELEASE_URL);
    clicked
}

/// Linux (DRV-M4): se installs no system package; the window names what is
/// missing and the package command, like `se drive runtime`. Never `true`.
#[cfg(target_os = "linux")]
pub(super) fn install_controls(ui: &mut egui::Ui, _enabled: bool, _link_label: &str) -> bool {
    let report = fuse_report();
    ui.vertical(|ui| {
        if report.ready {
            ui.label(format!(
                "FUSE bereit ({})",
                report
                    .fusermount3
                    .path
                    .as_deref()
                    .unwrap_or("fusermount3")
            ));
        } else {
            ui.label("FUSE ist nicht bereit:");
            for problem in &report.problems {
                ui.label(format!("• {problem}"));
            }
        }
        if let Some(seconds) = report.kernel_request_timeout_s.filter(|seconds| *seconds > 0) {
            ui.label(format!(
                "Hinweis: Der Kernel bricht ein Laufwerk ab, wenn eine Anfrage länger als {seconds} s dauert."
            ));
        }
        if report.wsl {
            ui.label(
                "Hinweis: Unter WSL zeigt der Windows-Explorer Laufwerke nur unter /mnt/wsl.",
            );
        }
    });
    false
}

/// The runtime report, read at most every 10 s (it reads system files).
#[cfg(target_os = "linux")]
fn fuse_report() -> crate::mount::FuseRuntimeReport {
    use std::sync::Mutex;
    use std::time::Duration;

    static CACHED: Mutex<Option<(Instant, crate::mount::FuseRuntimeReport)>> = Mutex::new(None);
    let mut cached = CACHED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match cached.as_ref() {
        Some((read, report)) if read.elapsed() < Duration::from_secs(10) => report.clone(),
        _ => {
            let report = crate::mount::fuse_runtime_report(false);
            *cached = Some((Instant::now(), report.clone()));
            report
        }
    }
}

pub(super) fn present_install_outcome(
    app: &mut App,
    outcome: crate::mount::DriveRuntimeInstallOutcome,
) {
    let message = outcome.message();
    if outcome.is_failure() {
        app.error_msg = Some(message);
    } else {
        app.notice = Some((message, Instant::now()));
    }
}
