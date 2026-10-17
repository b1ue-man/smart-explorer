//! Registration ledger and removal plan of se Desktop (C5.4 `ledger`, C5.7).
//!
//! The ledger lists what an activation applied. Disabling and removing delete
//! every ledger entry of the platform and, even without a ledger, every fixed
//! name of this module. A foreign GNOME extension that se switched off for
//! the take-over (C2.5) is switched on again only with its ledger entry.
use serde::{Deserialize, Serialize};

use super::state::Platform;

/// Windows `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value of the
/// shell (`"<GUI>" --desktop-shell`).
pub(crate) const RUN_VALUE_NAME: &str = "SmartExplorerDesktop";
/// CLSID of the navigation pane root "Smart Explorer Geräte" (C1.4); fixed
/// for all versions, documented in `docs/oberflaechen/desktop-launcher.md`.
pub(crate) const DESKTOP_NAV_CLSID: &str = "{174877FD-193E-42A6-936E-60CCB4C8F1C7}";
/// Linux XDG autostart file of the shell.
pub(crate) const AUTOSTART_FILE_NAME: &str = "smart-explorer-desktop.desktop";
/// UUID of the se GNOME Shell extension (C2.4).
pub(crate) const GNOME_EXTENSION_UUID: &str = "se-desktop@smartexplorer.app";
/// The jump list belongs to the GUI image; only its processes delete it (C1.5).
pub(crate) const JUMP_LIST_TARGET: &str = "Smart Explorer.exe";

/// Kind of a registration (`ledger[].kind` of `state.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Registration {
    RunValue,
    AutostartFile,
    NavPaneClsid,
    JumpList,
    GnomeExtensionFiles,
    GnomeExtensionEnabled,
    ForeignExtensionDisabled,
}

impl Registration {
    pub(crate) const ALL: [Registration; 7] = [
        Registration::RunValue,
        Registration::AutostartFile,
        Registration::NavPaneClsid,
        Registration::JumpList,
        Registration::GnomeExtensionFiles,
        Registration::GnomeExtensionEnabled,
        Registration::ForeignExtensionDisabled,
    ];

    /// `ledger[].kind` of `state.json` and `registrations[].kind` of the status.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Registration::RunValue => "run_value",
            Registration::AutostartFile => "autostart_file",
            Registration::NavPaneClsid => "nav_pane_clsid",
            Registration::JumpList => "jump_list",
            Registration::GnomeExtensionFiles => "gnome_extension_files",
            Registration::GnomeExtensionEnabled => "gnome_extension_enabled",
            Registration::ForeignExtensionDisabled => "foreign_extension_disabled",
        }
    }

    /// The platform on which this registration exists.
    pub(crate) fn platform(self) -> Platform {
        match self {
            Registration::RunValue | Registration::NavPaneClsid | Registration::JumpList => {
                Platform::Windows
            }
            Registration::AutostartFile
            | Registration::GnomeExtensionFiles
            | Registration::GnomeExtensionEnabled
            | Registration::ForeignExtensionDisabled => Platform::Linux,
        }
    }

    /// Removal order: the autostart first (nothing starts again), the own
    /// extension before a foreign desktop extension is switched back on.
    fn removal_rank(self) -> u8 {
        match self {
            Registration::RunValue => 0,
            Registration::AutostartFile => 1,
            Registration::NavPaneClsid => 2,
            Registration::JumpList => 3,
            Registration::GnomeExtensionEnabled => 4,
            Registration::GnomeExtensionFiles => 5,
            Registration::ForeignExtensionDisabled => 6,
        }
    }
}

/// One applied registration (`ledger[]` of `state.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LedgerEntry {
    pub(crate) kind: Registration,
    /// Name, path or UUID of what was applied.
    pub(crate) target: String,
    pub(crate) applied_at: i64,
}

/// One trace the os layer removes or reports as `residue`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Trace {
    pub(crate) kind: Registration,
    pub(crate) target: String,
}

/// Fixed names of this module, removed even when the ledger is lost. A
/// foreign extension is never in this list (C5.7).
pub(crate) const KNOWN_TRACES: [(Registration, &str); 6] = [
    (Registration::RunValue, RUN_VALUE_NAME),
    (Registration::NavPaneClsid, DESKTOP_NAV_CLSID),
    (Registration::JumpList, JUMP_LIST_TARGET),
    (Registration::AutostartFile, AUTOSTART_FILE_NAME),
    (Registration::GnomeExtensionEnabled, GNOME_EXTENSION_UUID),
    (Registration::GnomeExtensionFiles, GNOME_EXTENSION_UUID),
];

/// What `se desktop disable` and `se desktop remove` delete on `platform`:
/// every fixed name plus every ledger entry of this platform, each once, in
/// removal order. Foreign values under the same keys are the os layer's to
/// keep (pattern `unregister_verb`).
pub(crate) fn removal_plan(platform: Platform, ledger: &[LedgerEntry]) -> Vec<Trace> {
    let mut plan: Vec<Trace> = KNOWN_TRACES
        .iter()
        .filter(|(kind, _)| kind.platform() == platform)
        .map(|(kind, target)| Trace {
            kind: *kind,
            target: (*target).to_string(),
        })
        .collect();
    for entry in ledger
        .iter()
        .filter(|entry| entry.kind.platform() == platform)
    {
        let trace = Trace {
            kind: entry.kind,
            target: entry.target.clone(),
        };
        if !plan.contains(&trace) {
            plan.push(trace);
        }
    }
    plan.sort_by(|left, right| {
        left.kind
            .removal_rank()
            .cmp(&right.kind.removal_rank())
            .then_with(|| left.target.cmp(&right.target))
    });
    plan
}
