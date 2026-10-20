//! The GNOME part of Einstellungen → Integration → „se Desktop“ (U34-17,
//! C2.5): while an Ubuntu desktop-icons extension (DING) is active and the
//! setting is `keep`, the part "Desktopfläche" is unavailable; this offers
//! the button „Übernehmen“. Its click runs `se desktop config
//! --foreign-desktop-icons take-over` on the same step as the command line
//! (`Control::configure` in `with_control`), nothing of its own. After the
//! first activation of the extension the page says that it works after the
//! next login.
use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::desktop::core::config::{ConfigChange, ConfigContext, ForeignDesktopIcons};
use crate::desktop::os::control::Control;
use crate::desktop::os::status::{PartStatus, StatusReport};
use crate::desktop::os::{feed_sources, platform, Session};

/// The `unavailable` code of the part while the icons of another program
/// block it (`Reason::ForeignDesktopIcons`).
const BLOCKED_BY_ICONS: &str = "foreign_desktop_icons";

/// What is said above the button.
pub(in crate::app) const TAKE_OVER_TEXT: &str =
    "Ubuntu-Desktopsymbole (DING) sind aktiv. se Desktop \
zeigt die Dateien Ihres Desktop-Ordners selbst, wenn Sie die Übernahme erlauben. Beim Deaktivieren \
schaltet se DING wieder ein.";
pub(in crate::app) const RELOGIN_TEXT: &str = "Wirkt nach erneuter Anmeldung.";

/// Whether the offer shows for `part`.
pub(in crate::app) fn offers_take_over(part: &PartStatus) -> bool {
    part.unavailable.as_deref() == Some(BLOCKED_BY_ICONS)
}

/// Whether the page says "Wirkt nach erneuter Anmeldung."
pub(in crate::app) fn needs_relogin(report: &StatusReport) -> bool {
    report.extension_state == Some("installed_needs_relogin")
}

/// The offer and the note under the line of the part "Desktopfläche"; `true`
/// when the button was clicked.
pub(in crate::app) fn ui_gnome(
    ui: &mut egui::Ui,
    report: &StatusReport,
    part: &PartStatus,
    enabled: bool,
) -> bool {
    let mut clicked = false;
    if offers_take_over(part) {
        ui.label(
            RichText::new(TAKE_OVER_TEXT)
                .small()
                .color(theme::muted(ui)),
        );
        clicked = ui
            .add_enabled(enabled, egui::Button::new("Übernehmen"))
            .clicked();
    }
    if needs_relogin(report) {
        ui.label(RichText::new(RELOGIN_TEXT).small().color(theme::muted(ui)));
    }
    clicked
}

/// The step behind the button: the change of `se desktop config`.
pub(in crate::app) fn take_over(control: &mut Control<'_>) -> Result<String, String> {
    let change = ConfigChange {
        foreign_desktop_icons: Some(ForeignDesktopIcons::TakeOver),
        ..ConfigChange::default()
    };
    let stored = control.dir.read_config().ok().flatten();
    let context = ConfigContext {
        platform: platform::PLATFORM,
        transparent: Session::probe(stored.as_ref()).transparent,
        device_tree_mounted: feed_sources::device_tree_now().mounted(),
    };
    control
        .configure(&change, &context)
        .map(|_| "Desktopsymbole werden übernommen".to_string())
        .map_err(|error| error.to_string())
}
