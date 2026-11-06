//! Rights of one connection, chosen in one step (plan 2026-10-31 §F, F1–F6).
//!
//! [`ConnectionRights`] is what a connected device may do on this host
//! through one relation: which Direct folders it sees (F2), whether it may
//! write there, and the decisions of the compact menu for commands,
//! terminal and the screen. [`RightsPreset`] are the named fillings of that
//! menu (F4); every surface (desktop, Android, CLI, MCP, bridge, IPC) uses
//! these keys and labels.
//!
//! Mapping to the rights of `bestand-verbinden.md` §3:
//! - R1 folder list → [`FolderChoice`] (per device, stored as
//!   `ShareExportConfig::device_folders`); R2/R3 (read-only or read-write per
//!   folder, system locations) stay settings of the export itself.
//! - R4 write per device → `write` (rooms: R5 `members_may_write`).
//! - R8 commands, R9 terminal, R12 screen view, R13 screen control → the
//!   four decisions; R13 „Erlauben“ includes R14 apps, R15 login, R16 local
//!   services and R17 the host's own RDP service (`access_steering.rs`).
//! - R10 account → `account`; R18 end → `valid_until`.
//! - R11 program list, R16 explicit TCP rules, R20 share-back, R19 request
//!   policy and R21 connect/open automatically are not part of a preset;
//!   they keep their own settings („Mehr“ in the menu).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::share::access_grant::{
    AccessGrantChange, AccountProfile, Capability, CapabilityDecision, Expiry,
};

/// The capabilities the compact menu shows as switches.
pub const MENU_CAPABILITIES: [Capability; 4] = [
    Capability::ExecCommands,
    Capability::ExecTerminal,
    Capability::ScreenView,
    Capability::ScreenControl,
];

/// Named fillings of the compact rights menu (F4). A preset fills the
/// switches; every switch stays changeable on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RightsPreset {
    /// „Keine Rechte“: connected, nothing allowed.
    None,
    /// „Nur lesen“: every Direct folder, read only.
    ReadOnly,
    /// „Dateien lesen und schreiben“.
    ReadWrite,
    /// „Dateien und Codeausführung“: read and write, commands and terminal.
    FilesExec,
    /// „Alles“: files read and write, commands, terminal, desktop view and
    /// control.
    All,
}

impl RightsPreset {
    pub const ALL: [RightsPreset; 5] = [
        Self::None,
        Self::ReadOnly,
        Self::ReadWrite,
        Self::FilesExec,
        Self::All,
    ];

    /// Key on the CLI, in JSON, on the bridge and in IPC.
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
            Self::FilesExec => "files-exec",
            Self::All => "all",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.key() == key)
    }

    /// Name shown in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Keine Rechte",
            Self::ReadOnly => "Nur lesen",
            Self::ReadWrite => "Dateien lesen und schreiben",
            Self::FilesExec => "Dateien und Codeausführung",
            Self::All => "Alles",
        }
    }

    /// One sentence of what the preset allows.
    pub fn description(self) -> &'static str {
        match self {
            Self::None => {
                "Verbunden, aber ohne Ordner, Schreiben, Befehle, Terminal und Bildschirm."
            }
            Self::ReadOnly => {
                "Alle Direkt-Ordner lesen; kein Schreiben, keine Befehle, kein Terminal, kein Bildschirm."
            }
            Self::ReadWrite => {
                "Alle Direkt-Ordner lesen und schreiben (auch löschen und Papierkorb); keine Befehle, kein Terminal, kein Bildschirm."
            }
            Self::FilesExec => {
                "Alle Direkt-Ordner lesen und schreiben, dazu Befehle und Terminal als mein Konto; kein Bildschirm."
            }
            Self::All => {
                "Alle Direkt-Ordner lesen und schreiben, Befehle, Terminal, Desktop sehen und steuern (schließt Programme, Anmeldung, lokale Dienste und den eigenen Remote Desktop ein)."
            }
        }
    }

    /// The exact rights of the preset.
    pub fn rights(self) -> ConnectionRights {
        let allow = CapabilityDecision::Allow;
        let files = |write| ConnectionRights {
            folders: FolderChoice::All,
            write,
            ..ConnectionRights::none()
        };
        match self {
            Self::None => ConnectionRights::none(),
            Self::ReadOnly => files(false),
            Self::ReadWrite => files(true),
            Self::FilesExec => ConnectionRights {
                commands: allow,
                terminal: allow,
                ..files(true)
            },
            Self::All => ConnectionRights {
                commands: allow,
                terminal: allow,
                screen_view: allow,
                screen_control: allow,
                ..files(true)
            },
        }
    }
}

/// One preset as every surface lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetView {
    pub key: String,
    pub label: String,
    pub description: String,
    pub rights: ConnectionRights,
}

/// Every preset in menu order.
pub fn preset_views() -> Vec<PresetView> {
    RightsPreset::ALL
        .into_iter()
        .map(|preset| PresetView {
            key: preset.key().into(),
            label: preset.label().into(),
            description: preset.description().into(),
            rights: preset.rights(),
        })
        .collect()
}

/// Which Direct folders a device sees (F2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderChoice {
    /// No folder and no saved connection.
    #[default]
    None,
    /// Every Direct folder and saved connection, also ones added later (the
    /// meaning every grant without a selection keeps). For a room,
    /// confirming copies the Direct folders of that moment into the room's
    /// exports (`connection_rights_rooms.rs`); later Direct folders are not
    /// added (G5, review 2026-10-31): a folder added for one device would
    /// otherwise open to every member of every room with „Alle“, members
    /// this host never confirmed one by one. The surfaces say so (desktop
    /// and Android folder row, `se share grants confirm --room`).
    All,
    /// Exactly these Direct folders (`SharedRoot::path`), and the exported
    /// saved connections (`/Verbindungen`) when `connections` is set.
    Selected {
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default)]
        connections: bool,
    },
}

/// The rights of one connection as the compact menu shows them (F3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRights {
    #[serde(default)]
    pub folders: FolderChoice,
    /// May write where a folder is read-write (includes delete and trash).
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub commands: CapabilityDecision,
    #[serde(default)]
    pub terminal: CapabilityDecision,
    #[serde(default)]
    pub screen_view: CapabilityDecision,
    #[serde(default)]
    pub screen_control: CapabilityDecision,
    /// Unix seconds (host clock) from which commands, terminal and screen
    /// end; `None`: no end. Files are not bound to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<i64>,
    /// The account commands and the terminal run as (R10).
    #[serde(default)]
    pub account: AccountProfile,
}

impl Default for ConnectionRights {
    fn default() -> Self {
        Self::none()
    }
}

impl ConnectionRights {
    /// „Keine Rechte“.
    pub fn none() -> Self {
        Self {
            folders: FolderChoice::None,
            write: false,
            commands: CapabilityDecision::Off,
            terminal: CapabilityDecision::Off,
            screen_view: CapabilityDecision::Off,
            screen_control: CapabilityDecision::Off,
            valid_until: None,
            account: AccountProfile::Own,
        }
    }

    /// The preset these rights are exactly, if any.
    pub fn preset(&self) -> Option<RightsPreset> {
        RightsPreset::ALL
            .into_iter()
            .find(|preset| preset.rights() == *self)
    }

    pub fn decision(&self, capability: Capability) -> CapabilityDecision {
        match capability {
            Capability::ExecCommands => self.commands,
            Capability::ExecTerminal => self.terminal,
            Capability::ScreenView => self.screen_view,
            Capability::ScreenControl => self.screen_control,
            _ => CapabilityDecision::Off,
        }
    }

    pub fn set_decision(&mut self, capability: Capability, decision: CapabilityDecision) {
        match capability {
            Capability::ExecCommands => self.commands = decision,
            Capability::ExecTerminal => self.terminal = decision,
            Capability::ScreenView => self.screen_view = decision,
            Capability::ScreenControl => self.screen_control = decision,
            _ => {}
        }
    }

    /// Whether these rights allow anything at all.
    pub fn grants_anything(&self) -> bool {
        self.folders != FolderChoice::None
            || self.write
            || MENU_CAPABILITIES
                .into_iter()
                .any(|capability| self.decision(capability).is_on())
    }

    /// Rules that hold on every host: valid account name and end, folder
    /// paths not empty and not repeated. Host-specific checks (another
    /// account needs a root worker) belong to the OS layer.
    pub fn validate(&self) -> Result<(), String> {
        self.account.validate()?;
        if self.valid_until.is_some_and(|end| end <= 0) {
            return Err("the end of the rights must be a time after 1970".into());
        }
        if let FolderChoice::Selected { paths, .. } = &self.folders {
            let mut seen = std::collections::BTreeSet::new();
            for path in paths {
                if path.trim().is_empty() {
                    return Err("a selected folder must not be empty".into());
                }
                if !seen.insert(path.as_str()) {
                    return Err(format!("folder selected twice: {path}"));
                }
            }
        }
        Ok(())
    }

    /// The change that sets the four menu capabilities, account and end
    /// exactly. Program list, explicit app/login/TCP rights and their
    /// constraints stay; a terminal or full commands clear the program list
    /// only when the terminal is on (a program list allows no terminal).
    pub fn access_change(&self) -> AccessGrantChange {
        let capabilities = MENU_CAPABILITIES
            .into_iter()
            .map(|capability| (capability, self.decision(capability)))
            .collect::<BTreeMap<_, _>>();
        AccessGrantChange {
            account: Some(self.account.clone()),
            capabilities,
            programs: self.terminal.is_on().then(Vec::new),
            expiry: Some(match self.valid_until {
                Some(end) => Expiry::At(end),
                None => Expiry::None,
            }),
            constraints: None,
        }
    }

    /// Short German summary for text output and notices.
    pub fn summary(&self) -> String {
        if let Some(preset) = self.preset() {
            return preset.label().to_string();
        }
        let mut parts = vec![match &self.folders {
            FolderChoice::None => "keine Ordner".to_string(),
            FolderChoice::All => "alle Ordner".to_string(),
            FolderChoice::Selected { paths, connections } => format!(
                "{} Ordner{}",
                paths.len(),
                if *connections {
                    " und Verbindungen"
                } else {
                    ""
                }
            ),
        }];
        parts.push(if self.write { "schreiben" } else { "nur lesen" }.into());
        for (capability, name) in [
            (Capability::ExecCommands, "Befehle"),
            (Capability::ExecTerminal, "Terminal"),
            (Capability::ScreenView, "Desktop sehen"),
            (Capability::ScreenControl, "Desktop steuern"),
        ] {
            match self.decision(capability) {
                CapabilityDecision::Off => {}
                CapabilityDecision::Ask => parts.push(format!("{name} mit Rückfrage")),
                CapabilityDecision::Allow => parts.push(name.to_string()),
            }
        }
        parts.join(", ")
    }
}

/// What has rights waiting for confirmation (F1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RightsTarget {
    /// The Direct grant of exactly this device and key.
    Direct {
        device_id: String,
        public_key: String,
    },
    /// The room with this local profile id (`RoomProfile::id`).
    Room { profile_id: String },
}

/// How the connection that waits for its rights came about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsOrigin {
    /// A request with this device's Direct code was accepted (by the user
    /// or automatically).
    Request,
    /// PIN pairing, either side.
    Pairing,
    /// „Auch meine Freigaben für dieses Gerät öffnen“ on a contact.
    ShareBack,
    /// The background repair completed a missing reverse relation.
    Repair,
    /// The device's grant now belongs to a new key.
    IdentityReplaced,
    /// A blocked or never-confirmed grant was allowed again, or a new
    /// request of a revoked or ignored device was accepted (W6).
    Readmitted,
    /// A room created on this device.
    RoomCreated,
    /// A room joined by code or PIN.
    RoomJoined,
}

impl RightsOrigin {
    pub fn key(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Pairing => "pairing",
            Self::ShareBack => "share_back",
            Self::Repair => "repair",
            Self::IdentityReplaced => "identity_replaced",
            Self::Readmitted => "readmitted",
            Self::RoomCreated => "room_created",
            Self::RoomJoined => "room_joined",
        }
    }
}

/// „Rechte offen“: a connection that exists without rights until the user
/// confirms (sets them) or cancels (keeps it without rights).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRights {
    pub target: RightsTarget,
    pub origin: RightsOrigin,
    /// Unix seconds when the connection came about.
    pub since: i64,
}

impl RightsTarget {
    pub fn is_direct(&self, device: &str) -> bool {
        matches!(self, Self::Direct { device_id, .. } if device_id == device)
    }

    pub fn is_room(&self, profile: &str) -> bool {
        matches!(self, Self::Room { profile_id } if profile_id == profile)
    }
}

#[cfg(test)]
#[path = "abgleich_task_connection_rights_tests.rs"]
mod abgleich_task_tests;

#[cfg(test)]
#[path = "rechte_task_pairing_tests.rs"]
mod rechte_task_pairing_tests;
