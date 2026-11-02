//! The capabilities this host offers (contract V7, E point 3): one entry per
//! row that the access dialogs, `se share grants access` and the Android
//! bridge show, so every surface lists the same capabilities in the same
//! order. Exec registers commands and terminal here; the screen strand (S4,
//! SCR-M2) adds its rows with
//! `crate::screen::core::catalog_entries::register_screen_entries(&mut
//! entries, facts)` in `capability_catalog`, under the condition
//! `facts.graphical_session`.
use serde::{Deserialize, Serialize};

use crate::share::access_grant::Capability;

/// State of the account helper (PRM-M6, PRM-M7) as surfaces report it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperState {
    /// This host switches accounts without a helper (Linux root worker), or
    /// only the own account is possible.
    NotNeeded,
    NotInstalled,
    Installed,
    /// Installed with another protocol version; it needs an update.
    Outdated,
    /// This platform has no account helper (Android).
    Unavailable,
}

/// The account helper with its version and a reason text when it fails.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperStatus {
    pub state: HelperState,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// What the catalog depends on: exactly these two facts of the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFacts {
    /// A graphical session exists in which the screen can be shown.
    pub graphical_session: bool,
    pub account_helper: HelperStatus,
}

/// One capability row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityEntry {
    pub capability: Capability,
    /// Label of the desktop and Android dialogs.
    pub label: &'static str,
    /// What allowing it lets the device do (desktop and Android).
    pub description: &'static str,
    /// Option of `se share grants access` without the dashes.
    pub cli_option: &'static str,
    /// Key of the Android bridge (`share.setAccess`, `capabilities`).
    pub bridge_key: &'static str,
    /// The account profile ("Ausführen als") applies to it.
    pub uses_account: bool,
}

/// The rows of this host, Exec first, then the screen rows of S4 (only with
/// a graphical session).
pub fn capability_catalog(facts: &HostFacts) -> Vec<CapabilityEntry> {
    let mut entries = vec![
        CapabilityEntry {
            capability: Capability::ExecCommands,
            label: "Befehle",
            description: "Programme und Shell-Befehle starten",
            cli_option: "commands",
            bridge_key: "execCommands",
            uses_account: true,
        },
        CapabilityEntry {
            capability: Capability::ExecTerminal,
            label: "Terminal",
            description: "Ein Terminal mit der Login-Shell öffnen",
            cli_option: "terminal",
            bridge_key: "execTerminal",
            uses_account: true,
        },
    ];
    crate::screen::core::catalog_entries::register_screen_entries(&mut entries, facts);
    entries
}

/// Add only contextual operations whose real host adapter is installed and
/// usable. Callers must obtain `available` from their adapters, not assume
/// support merely because AccessV2 types can be parsed.
pub fn capability_catalog_with_contextual(
    facts: &HostFacts,
    available: &[Capability],
) -> Vec<CapabilityEntry> {
    let mut entries = capability_catalog(facts);
    for (capability, label, description, cli_option, bridge_key) in [
        (
            Capability::AppLaunch,
            "Apps starten",
            "Freigegebene Apps in erlaubten Benutzersitzungen starten",
            "app-launch",
            "appLaunch",
        ),
        (
            Capability::LoginAccess,
            "Anmeldezugang",
            "Administrativ freigegebene Benutzer- und Anmeldesitzungen erreichen",
            "login-access",
            "loginAccess",
        ),
        (
            Capability::TcpForward,
            "TCP weiterleiten",
            "Freigegebene TCP-Ziele mit konkreter Richtung und Portmenge erreichen",
            "tcp-forward",
            "tcpForward",
        ),
    ] {
        if available.contains(&capability) {
            entries.push(CapabilityEntry {
                capability,
                label,
                description,
                cli_option,
                bridge_key,
                uses_account: capability.uses_account(),
            });
        }
    }
    entries
}
