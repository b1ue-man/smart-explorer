//! Registrations of se Desktop on Linux (C5.7): the shell's XDG autostart
//! file, the GNOME extension folder and its UUID in `enabled-extensions`
//! (written with DSK-M8), a foreign desktop-icons extension switched off for
//! the take-over (restored only with its ledger entry, C2.5), and the
//! runtime files below `$XDG_RUNTIME_DIR/smart-explorer/`. Only this
//! module's fixed names are touched; that folder itself also holds drive
//! mount points (S2) and always stays.
use std::path::{Path, PathBuf};

use super::autostart;
use super::gnome_settings::{extension_installed, valid_uuid, GnomeSettings, DISABLED, ENABLED};
use crate::desktop::core::ledger::{
    LedgerEntry, Registration, Trace, AUTOSTART_FILE_NAME, GNOME_EXTENSION_UUID,
};
use crate::desktop::core::state::Platform;
use crate::desktop::os::control::{AutostartState, Integration};
use crate::desktop::os::store::unix_now;

/// Runtime files of the GNOME channel (C2.4, C5.5).
const RUNTIME_FILES: [&str; 4] = [
    "desktop.sock",
    "desktop.token",
    "desktop-gnome.json",
    "desktop-extension.json",
];

/// Folders the Linux traces live in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPaths {
    /// `$XDG_CONFIG_HOME/autostart`.
    pub(crate) autostart_dir: PathBuf,
    /// `$XDG_DATA_HOME/gnome-shell/extensions`.
    pub(crate) extensions_dir: PathBuf,
    /// `$XDG_RUNTIME_DIR/smart-explorer`, when the session has one.
    pub(crate) runtime_dir: Option<PathBuf>,
}

fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

impl LinuxPaths {
    pub(crate) fn from_env() -> LinuxPaths {
        let home = absolute_env("HOME").unwrap_or_else(std::env::temp_dir);
        let config = absolute_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data = absolute_env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        LinuxPaths {
            autostart_dir: config.join("autostart"),
            extensions_dir: data.join("gnome-shell").join("extensions"),
            runtime_dir: absolute_env("XDG_RUNTIME_DIR").map(|dir| dir.join("smart-explorer")),
        }
    }
}

pub(crate) struct LinuxIntegration {
    paths: LinuxPaths,
    gnome: GnomeSettings,
}

impl LinuxIntegration {
    pub(crate) fn new(_gui: Option<PathBuf>) -> LinuxIntegration {
        LinuxIntegration::with(LinuxPaths::from_env(), GnomeSettings::system())
    }

    pub(crate) fn with(paths: LinuxPaths, gnome: GnomeSettings) -> LinuxIntegration {
        LinuxIntegration { paths, gnome }
    }

    /// Task tests: every folder below `root`, no GNOME settings.
    #[cfg(test)]
    pub(crate) fn for_task(root: &Path) -> LinuxIntegration {
        LinuxIntegration::with(super::task_fixtures::paths(root), GnomeSettings::none())
    }

    fn extension_dir(&self) -> PathBuf {
        self.paths.extensions_dir.join(GNOME_EXTENSION_UUID)
    }

    fn own_uuid_enabled(&self) -> bool {
        self.gnome
            .read(ENABLED)
            .is_some_and(|list| list.iter().any(|uuid| uuid == GNOME_EXTENSION_UUID))
    }

    /// A foreign extension se switched off is still off and still installed.
    fn foreign_still_disabled(&self, uuid: &str) -> bool {
        valid_uuid(uuid)
            && extension_installed(&self.paths.extensions_dir, uuid)
            && self
                .gnome
                .read(ENABLED)
                .is_some_and(|list| !list.iter().any(|item| item == uuid))
    }

    fn remove_own_uuid(&self) -> Result<(), String> {
        let Some(mut list) = self.gnome.read(ENABLED) else {
            return Ok(());
        };
        let before = list.len();
        list.retain(|uuid| uuid != GNOME_EXTENSION_UUID);
        if list.len() == before {
            return Ok(());
        }
        self.gnome.write(ENABLED, &list)
    }

    /// C2.5: switched on again only when it is still off; a user who
    /// switched it on or removed it meanwhile keeps that.
    fn restore_foreign(&self, uuid: &str) -> Result<(), String> {
        if !valid_uuid(uuid) {
            return Err(format!("ungültige Erweiterungskennung {uuid:?}"));
        }
        if !self.foreign_still_disabled(uuid) {
            return Ok(());
        }
        let mut enabled = self.gnome.read(ENABLED).unwrap_or_default();
        enabled.push(uuid.to_string());
        self.gnome.write(ENABLED, &enabled)?;
        if let Some(mut disabled) = self.gnome.read(DISABLED) {
            let before = disabled.len();
            disabled.retain(|item| item != uuid);
            if disabled.len() != before {
                self.gnome.write(DISABLED, &disabled)?;
            }
        }
        Ok(())
    }

    fn remove_extension_dir(&self) -> Result<(), String> {
        let dir = self.extension_dir();
        let result = match std::fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&dir),
            // A link or file under the fixed name: removed as itself.
            Ok(_) => std::fs::remove_file(&dir),
        };
        result.map_err(|error| format!("{}: {error}", dir.display()))
    }
}

impl Integration for LinuxIntegration {
    fn platform(&self) -> Platform {
        Platform::Linux
    }

    fn autostart_registration(&self) -> Registration {
        Registration::AutostartFile
    }

    fn apply(&mut self, registration: Registration, gui: &Path) -> Result<LedgerEntry, String> {
        match registration {
            Registration::AutostartFile => {
                autostart::write(&self.paths.autostart_dir, gui)
                    .map_err(|error| error.to_string())?;
                Ok(LedgerEntry {
                    kind: registration,
                    target: AUTOSTART_FILE_NAME.into(),
                    applied_at: unix_now(),
                })
            }
            other => Err(format!(
                "{} legt nur der se-Desktop-Wächter an",
                other.key()
            )),
        }
    }

    fn unapply(&mut self, trace: &Trace) -> Result<(), String> {
        let own = trace.target == GNOME_EXTENSION_UUID;
        match trace.kind {
            Registration::AutostartFile => {
                autostart::remove(&self.paths.autostart_dir, &trace.target)
                    .map_err(|error| error.to_string())
            }
            Registration::GnomeExtensionFiles if own => self.remove_extension_dir(),
            Registration::GnomeExtensionEnabled if own => self.remove_own_uuid(),
            Registration::GnomeExtensionFiles | Registration::GnomeExtensionEnabled => Err(
                format!("fremde Erweiterung {:?} wird nicht verändert", trace.target),
            ),
            Registration::ForeignExtensionDisabled => self.restore_foreign(&trace.target),
            // Windows registrations do not exist here.
            _ => Ok(()),
        }
    }

    fn is_applied(&self, trace: &Trace) -> bool {
        let own = trace.target == GNOME_EXTENSION_UUID;
        match trace.kind {
            Registration::AutostartFile => {
                trace.target == AUTOSTART_FILE_NAME && autostart::present(&self.paths.autostart_dir)
            }
            Registration::GnomeExtensionFiles => {
                own && std::fs::symlink_metadata(self.extension_dir()).is_ok()
            }
            Registration::GnomeExtensionEnabled => own && self.own_uuid_enabled(),
            Registration::ForeignExtensionDisabled => self.foreign_still_disabled(&trace.target),
            _ => false,
        }
    }

    fn autostart(&self) -> AutostartState {
        let dir = &self.paths.autostart_dir;
        if !autostart::present(dir) {
            AutostartState::Off
        } else if autostart::disabled_by_system(dir) {
            AutostartState::DisabledBySystem
        } else {
            AutostartState::On
        }
    }

    fn runtime_files(&self) -> Vec<PathBuf> {
        let Some(dir) = &self.paths.runtime_dir else {
            return Vec::new();
        };
        RUNTIME_FILES
            .iter()
            .map(|name| dir.join(name))
            .filter(|path| std::fs::symlink_metadata(path).is_ok())
            .collect()
    }
}
