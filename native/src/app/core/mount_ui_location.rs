//! „Einhängeort“ and „Beim Anmelden automatisch einbinden“ of the window
//! „Remote als Laufwerk“ (DRV-M3), moved out of `mount_ui_draft.rs`: the
//! drive letter choice of v0.5.182 unchanged (Windows), the folder field of
//! platforms that place a drive at a folder (Linux, reachable once DRV-M4
//! mounts drives there) and the option to mount the drive again at every
//! login.
use crate::app::mount_ui_helpers::drive_selection_label;
use crate::app::prelude::*;
use crate::app::theme;

const LOGIN_HINT: &str = "Smart Explorer bindet das Laufwerk nach jeder Anmeldung und nach einem Neustart des Hintergrunddienstes wieder ein, ohne dass du Smart Explorer öffnest. Auswerfen entfernt das Laufwerk samt dieser Einstellung.";

/// The location choices of one draft.
pub(crate) struct LocationDraft {
    pub(crate) drive: crate::mount::DriveSelection,
    custom_folder: bool,
    folder: String,
    pub(crate) auto_remount: bool,
}

impl Default for LocationDraft {
    fn default() -> Self {
        Self {
            drive: crate::mount::DriveSelection::Automatic,
            custom_folder: false,
            folder: String::new(),
            auto_remount: false,
        }
    }
}

impl LocationDraft {
    /// The folder the drive shall appear at; `None` keeps the default of the
    /// platform.
    fn chosen_folder(&self) -> Option<PathBuf> {
        (!super::DRIVES_USE_LETTERS && self.custom_folder)
            .then(|| PathBuf::from(self.folder.trim()))
    }

    /// Why the chosen folder cannot be stored, if it cannot.
    pub(crate) fn problem(&self) -> Option<String> {
        let folder = self.chosen_folder()?;
        crate::mount::MountConfig::check_location_directory(&folder)
            .err()
            .map(|error| error.to_string())
    }

    /// Applies the choices to a new drive definition.
    pub(crate) fn apply(
        &self,
        config: crate::mount::MountConfig,
    ) -> std::io::Result<crate::mount::MountConfig> {
        config
            .with_auto_remount(self.auto_remount)
            .with_location_directory(self.chosen_folder())
    }
}

/// The section „Einhängeort“ and the login option.
pub(crate) fn render_location(ui: &mut egui::Ui, draft: &mut LocationDraft) {
    if super::DRIVES_USE_LETTERS {
        render_drive_letter(ui, &mut draft.drive);
    } else {
        render_folder(ui, draft);
    }
    ui.checkbox(
        &mut draft.auto_remount,
        "Beim Anmelden automatisch einbinden",
    )
    .on_hover_text(LOGIN_HINT);
}

/// The drive letter choice of v0.5.182.
fn render_drive_letter(ui: &mut egui::Ui, drive: &mut crate::mount::DriveSelection) {
    ui.horizontal(|ui| {
        ui.label("Laufwerksbuchstabe:");
        egui::ComboBox::from_id_salt("mount_drive_letter")
            .selected_text(drive_selection_label(*drive))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    drive,
                    crate::mount::DriveSelection::Automatic,
                    "Automatisch",
                );
                for letter in
                    ('D'..='Z').filter_map(|letter| crate::mount::DriveLetter::parse(letter).ok())
                {
                    ui.selectable_value(
                        drive,
                        crate::mount::DriveSelection::Letter(letter),
                        letter.to_string(),
                    );
                }
            });
    });
}

fn render_folder(ui: &mut egui::Ui, draft: &mut LocationDraft) {
    ui.horizontal(|ui| {
        ui.label("Einhängeort:");
        ui.radio_value(
            &mut draft.custom_folder,
            false,
            "Standard (~/SmartExplorer)",
        );
        ui.radio_value(&mut draft.custom_folder, true, "Eigener Ordner");
    });
    if !draft.custom_folder {
        return;
    }
    ui.add(egui::TextEdit::singleline(&mut draft.folder).hint_text("/home/…/Ordner"))
        .on_hover_text("Absoluter Pfad eines leeren Ordners, der dir gehört");
    if let Some(problem) = draft.problem() {
        ui.colored_label(theme::danger(ui), problem);
    }
}
