//! The se GNOME Shell extension on disk (U34-11, U34-16, U34-17, U34-18): the
//! two embedded variants, their installation, the take-over of a foreign
//! desktop-icons extension and the state the status reports.
//!
//! Installation (the shell, at its start and when the session changes):
//! (1) the variant by the Shell's major version (45 to 48 → A, 49 to 51 → B);
//! (2) its files below `$XDG_DATA_HOME/gnome-shell/extensions/<uuid>/`, each
//! written atomically and rewritten when its SHA-256 differs from the
//! embedded text; (3) the UUID in `org.gnome.shell enabled-extensions`;
//! (4) `EnableExtension` over the bus. A `false` answer means that the running
//! Shell does not know the new extension yet: it loads after the next login
//! (`installed_needs_relogin`). A UUID the user put in `disabled-extensions`
//! and `disable-user-extensions = true` are reported and never changed.
//! Every step that changed something is a ledger entry; disabling and removing
//! undoes exactly these and the fixed names (`gnome_settings`, `integration`).
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::gnome_control::{ExtensionReport, RuntimeFiles};
use super::gnome_dbus::{SessionBus, ShellBus, STATE_ENABLED};
use super::gnome_settings::{GnomeSettings, DISABLED, ENABLED};
use super::integration::LinuxPaths;
use crate::desktop::core::ledger::{LedgerEntry, Registration, GNOME_EXTENSION_UUID};
use crate::desktop::core::mechanism::{DisplayServer, GnomeVariant, SessionFacts};
use crate::desktop::os::store::unix_now;

const FILES_A: [(&str, &str); 2] = [
    (
        "metadata.json",
        include_str!("../../../../assets/gnome-extension/a/metadata.json"),
    ),
    (
        "extension.js",
        include_str!("../../../../assets/gnome-extension/a/extension.js"),
    ),
];
const FILES_B: [(&str, &str); 2] = [
    (
        "metadata.json",
        include_str!("../../../../assets/gnome-extension/b/metadata.json"),
    ),
    (
        "extension.js",
        include_str!("../../../../assets/gnome-extension/b/extension.js"),
    ),
];

/// The variant for a Shell major version; `None` below 45 and above 51.
pub(crate) fn variant_for(major: u32) -> Option<GnomeVariant> {
    match major {
        45..=48 => Some(GnomeVariant::A),
        49..=51 => Some(GnomeVariant::B),
        _ => None,
    }
}

/// `(file name, content)` of a variant.
pub(crate) fn bundle(variant: GnomeVariant) -> &'static [(&'static str, &'static str)] {
    match variant {
        GnomeVariant::A => &FILES_A,
        GnomeVariant::B => &FILES_B,
    }
}

/// The folder of the extension.
pub(crate) fn extension_dir(paths: &LinuxPaths) -> PathBuf {
    paths.extensions_dir.join(GNOME_EXTENSION_UUID)
}

fn digest(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

/// All files of `variant` are there with the embedded content (SHA-256).
pub(crate) fn files_current(dir: &Path, variant: GnomeVariant) -> bool {
    bundle(variant).iter().all(|(name, text)| {
        std::fs::read(dir.join(name)).is_ok_and(|found| digest(&found) == digest(text.as_bytes()))
    })
}

/// The extension's files exist (any content).
pub(crate) fn files_present(dir: &Path) -> bool {
    bundle(GnomeVariant::A)
        .iter()
        .all(|(name, _)| dir.join(name).is_file())
}

/// Writes the files of `variant`, each through a helper file and a rename.
pub(crate) fn write_files(dir: &Path, variant: GnomeVariant) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for (name, text) in bundle(variant) {
        let target = dir.join(name);
        let staged = dir.join(format!(".{name}.se-new"));
        std::fs::write(&staged, text)
            .and_then(|()| std::fs::rename(&staged, &target))
            .map_err(|error| {
                let _ = std::fs::remove_file(&staged);
                format!("{}: {error}", target.display())
            })?;
    }
    Ok(())
}

/// How far the installation got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallOutcome {
    /// The running Shell has it enabled.
    Enabled,
    /// Written and enabled in the settings; the Shell loads it after the
    /// next login.
    NeedsRelogin,
    /// The user switched it off in the extension list; nothing was changed.
    DisabledByUser,
    /// `disable-user-extensions` is on; nothing was changed.
    UserExtensionsDisabled,
    /// No `gsettings` or no schema: only the files could be written.
    NoSettings,
}

/// What an installation did and the ledger entries it earned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstallReport {
    pub(crate) entries: Vec<LedgerEntry>,
    pub(crate) outcome: InstallOutcome,
}

fn entry(kind: Registration) -> LedgerEntry {
    LedgerEntry {
        kind,
        target: GNOME_EXTENSION_UUID.into(),
        applied_at: unix_now(),
    }
}

/// Steps 2 to 4 of the installation (see the module text) for `variant`.
pub(crate) fn install(
    paths: &LinuxPaths,
    gnome: &GnomeSettings,
    bus: &dyn ShellBus,
    variant: GnomeVariant,
) -> Result<InstallReport, String> {
    let dir = extension_dir(paths);
    if !files_current(&dir, variant) {
        write_files(&dir, variant)?;
    }
    let mut entries = vec![entry(Registration::GnomeExtensionFiles)];
    let report = |entries: Vec<LedgerEntry>, outcome| Ok(InstallReport { entries, outcome });
    if gnome.read_bool("disable-user-extensions") == Some(true) {
        return report(entries, InstallOutcome::UserExtensionsDisabled);
    }
    let disabled = gnome.read(DISABLED).unwrap_or_default();
    if disabled.iter().any(|uuid| uuid == GNOME_EXTENSION_UUID) {
        return report(entries, InstallOutcome::DisabledByUser);
    }
    let Some(mut enabled) = gnome.read(ENABLED) else {
        return report(entries, InstallOutcome::NoSettings);
    };
    if !enabled.iter().any(|uuid| uuid == GNOME_EXTENSION_UUID) {
        enabled.push(GNOME_EXTENSION_UUID.to_string());
        gnome.write(ENABLED, &enabled)?;
    }
    entries.push(entry(Registration::GnomeExtensionEnabled));
    let running = bus.list_extensions().is_some_and(|list| {
        list.iter()
            .any(|(uuid, state)| uuid == GNOME_EXTENSION_UUID && *state == STATE_ENABLED)
    });
    if running || bus.set_extension(GNOME_EXTENSION_UUID, true) == Ok(true) {
        return report(entries, InstallOutcome::Enabled);
    }
    report(entries, InstallOutcome::NeedsRelogin)
}

/// Disabling and removing: the running Shell is told (`DisableExtension`,
/// best effort), then the UUID leaves both lists.
pub(crate) fn remove_enabled(gnome: &GnomeSettings, bus: &dyn ShellBus) -> Result<(), String> {
    let _ = bus.set_extension(GNOME_EXTENSION_UUID, false);
    for key in [ENABLED, DISABLED] {
        let Some(mut list) = gnome.read(key) else {
            continue;
        };
        let before = list.len();
        list.retain(|uuid| uuid != GNOME_EXTENSION_UUID);
        if list.len() != before {
            gnome.write(key, &list)?;
        }
    }
    Ok(())
}

/// The extension's folder, also when only a link or a file has its name.
pub(crate) fn remove_files(dir: &Path) -> Result<(), String> {
    let result = match std::fs::symlink_metadata(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(dir),
        Ok(_) => std::fs::remove_file(dir),
    };
    result.map_err(|error| format!("{}: {error}", dir.display()))
}

/// C2.5: switches the foreign desktop-icons extension off; the ledger entry
/// that lets `disable` and `remove` switch it on again.
pub(crate) fn take_over(bus: &dyn ShellBus, uuid: &str) -> Result<LedgerEntry, String> {
    match bus.set_extension(uuid, false)? {
        true => Ok(LedgerEntry {
            kind: Registration::ForeignExtensionDisabled,
            target: uuid.to_string(),
            applied_at: unix_now(),
        }),
        false => Err(format!(
            "Die GNOME Shell hat das Abschalten von {uuid} nicht angenommen"
        )),
    }
}

/// The values of the `extension_state` of the status (U34-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExtensionState {
    NotInstalled,
    Installed,
    InstalledNeedsRelogin,
    Running,
    DisabledByUser,
    UserExtensionsDisabled,
    UnsupportedShellVersion,
    Halted,
}

impl ExtensionState {
    pub(crate) fn key(self) -> &'static str {
        match self {
            ExtensionState::NotInstalled => "not_installed",
            ExtensionState::Installed => "installed",
            ExtensionState::InstalledNeedsRelogin => "installed_needs_relogin",
            ExtensionState::Running => "running",
            ExtensionState::DisabledByUser => "disabled_by_user",
            ExtensionState::UserExtensionsDisabled => "user_extensions_disabled",
            ExtensionState::UnsupportedShellVersion => "unsupported_shell_version",
            ExtensionState::Halted => "halted",
        }
    }
}

/// What the state is made of.
#[derive(Clone, Debug, Default)]
pub(crate) struct StateInputs {
    /// GNOME on Wayland without the layer protocol: the extension's session.
    pub(crate) applicable: bool,
    pub(crate) shell_major: Option<u32>,
    pub(crate) files_present: bool,
    pub(crate) user_extensions_disabled: bool,
    pub(crate) in_disabled_list: bool,
    pub(crate) in_enabled_list: bool,
    /// The running Shell lists the extension as enabled (`None`: the bus did
    /// not answer).
    pub(crate) shell_runs_it: Option<bool>,
    pub(crate) report: Option<ExtensionReport>,
}

/// The state, in this order: not applicable → none; an unlisted Shell
/// version; a halt; the two switches of the user; no files; a placed window;
/// enabled but unknown to the running Shell; installed.
pub(crate) fn extension_state(inputs: &StateInputs) -> Option<ExtensionState> {
    if !inputs.applicable {
        return None;
    }
    if let Some(major) = inputs.shell_major {
        if variant_for(major).is_none() {
            return Some(ExtensionState::UnsupportedShellVersion);
        }
    } else {
        return None;
    }
    if inputs
        .report
        .as_ref()
        .is_some_and(|report| report.halted.is_some())
    {
        return Some(ExtensionState::Halted);
    }
    Some(if inputs.user_extensions_disabled {
        ExtensionState::UserExtensionsDisabled
    } else if inputs.in_disabled_list {
        ExtensionState::DisabledByUser
    } else if !inputs.files_present {
        ExtensionState::NotInstalled
    } else if inputs
        .report
        .as_ref()
        .is_some_and(ExtensionReport::handles_a_window)
    {
        ExtensionState::Running
    } else if inputs.in_enabled_list && inputs.shell_runs_it == Some(false) {
        ExtensionState::InstalledNeedsRelogin
    } else {
        ExtensionState::Installed
    })
}

/// The state in this session (see [`StateInputs`]); it reads the settings and
/// asks the running Shell, but only in the session the extension is for
/// (GNOME on Wayland without the layer protocol).
pub(crate) fn current_state(facts: &SessionFacts) -> Option<&'static str> {
    let gnome = facts
        .current_desktop
        .iter()
        .any(|part| part.eq_ignore_ascii_case("GNOME"));
    let applicable =
        gnome && facts.display == DisplayServer::Wayland && facts.layer_shell_version.is_none();
    if !applicable || facts.gnome_shell_major.is_none() {
        return None;
    }
    let paths = LinuxPaths::from_env();
    let settings = GnomeSettings::system();
    let uuid_in = |key: &str| {
        settings
            .read(key)
            .is_some_and(|list| list.iter().any(|known| known == GNOME_EXTENSION_UUID))
    };
    let listed = SessionBus.list_extensions();
    let inputs = StateInputs {
        applicable,
        shell_major: facts.gnome_shell_major,
        files_present: files_present(&extension_dir(&paths)),
        user_extensions_disabled: settings.read_bool("disable-user-extensions") == Some(true),
        in_disabled_list: uuid_in(DISABLED),
        in_enabled_list: uuid_in(ENABLED),
        shell_runs_it: listed.map(|list| {
            list.iter()
                .any(|(uuid, state)| uuid == GNOME_EXTENSION_UUID && *state == STATE_ENABLED)
        }),
        report: RuntimeFiles::from_env().and_then(|files| files.read_extension()),
    };
    extension_state(&inputs).map(ExtensionState::key)
}
