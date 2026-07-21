use std::time::Instant;

use super::App;

const DOKANY_RELEASE_URL: &str = "https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000";

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
