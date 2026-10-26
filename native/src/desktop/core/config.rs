//! `config.json` schema 1 of se Desktop (C5.4) with its rules. The options of
//! `se desktop config` (DSK-M3) arrive as a [`ConfigChange`] and are checked
//! against the facts of the session in a [`ConfigContext`].
use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) use super::folder_open::OpenFoldersWith;
use super::state::Platform;

pub(crate) const CONFIG_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackgroundKind {
    System,
    Color,
    Image,
    Transparent,
}

/// `background` of `config.json`: `value` is `null`, `#RRGGBB` or a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Background {
    pub(crate) kind: BackgroundKind,
    pub(crate) value: Option<String>,
}

impl Background {
    pub(crate) fn system() -> Background {
        Background {
            kind: BackgroundKind::System,
            value: None,
        }
    }

    /// `--background system|transparent|color:#RRGGBB|image:PFAD`.
    pub(crate) fn parse_cli(value: &str) -> Result<Background, ConfigError> {
        let (kind, rest) = if let Some(color) = value.strip_prefix("color:") {
            (BackgroundKind::Color, Some(color))
        } else if let Some(path) = value.strip_prefix("image:") {
            (BackgroundKind::Image, Some(path))
        } else {
            match value {
                "system" => (BackgroundKind::System, None),
                "transparent" => (BackgroundKind::Transparent, None),
                _ => {
                    return Err(ConfigError::InvalidBackground(format!(
                        "{value:?}; erwartet system, transparent, color:#RRGGBB oder image:PFAD"
                    )))
                }
            }
        };
        let background = Background {
            kind,
            value: rest.map(str::to_string),
        };
        background.validate()?;
        Ok(background)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let valid = match (self.kind, self.value.as_deref()) {
            (BackgroundKind::System | BackgroundKind::Transparent, None) => true,
            (BackgroundKind::Color, Some(color)) => is_hex_color(color),
            (BackgroundKind::Image, Some(path)) => {
                !path.is_empty() && !path.chars().any(char::is_control)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(ConfigError::InvalidBackground(format!(
                "{:?} mit Wert {:?}",
                self.kind, self.value
            )))
        }
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
}

/// Groups of the surface (C4.2), each can be hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Groups {
    pub(crate) desktop_files: bool,
    pub(crate) devices: bool,
    pub(crate) rooms: bool,
    pub(crate) vaults: bool,
    pub(crate) screens: bool,
}

/// Windows "Papierkorb" and "Dieser PC" as se settings (Ä5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SystemItems {
    pub(crate) recycle_bin: bool,
    pub(crate) this_pc: bool,
}

impl SystemItems {
    /// Start values at activation from Explorer's `HideDesktopIcons\NewStartPanel`
    /// values (0 shown, otherwise hidden). Without a value the se default holds:
    /// Papierkorb on, Dieser PC off (a se choice, not a Windows default).
    pub(crate) fn initial(
        recycle_bin_hidden: Option<u32>,
        this_pc_hidden: Option<u32>,
    ) -> SystemItems {
        let shown = |hidden: Option<u32>, default: bool| match hidden {
            Some(value) => value == 0,
            None => default,
        };
        SystemItems {
            recycle_bin: shown(recycle_bin_hidden, true),
            this_pc: shown(this_pc_hidden, false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OpenDevicesWith {
    App,
    FileManager,
}

/// GNOME desktop icons (DING) under the surface (C2.5): kept unless the user
/// agrees to the take-over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ForeignDesktopIcons {
    Keep,
    TakeOver,
}

/// Content of `config.json`; kept by `disable`, deleted by `remove`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DesktopConfig {
    pub(crate) schema: u32,
    pub(crate) background: Background,
    pub(crate) groups: Groups,
    pub(crate) system_items: SystemItems,
    pub(crate) open_devices_with: OpenDevicesWith,
    #[serde(default)]
    pub(crate) open_folders_with: OpenFoldersWith,
    pub(crate) foreign_desktop_icons: ForeignDesktopIcons,
}

impl DesktopConfig {
    /// The configuration of a first activation.
    pub(crate) fn initial(system_items: SystemItems) -> DesktopConfig {
        DesktopConfig {
            schema: CONFIG_SCHEMA,
            background: Background::system(),
            groups: Groups {
                desktop_files: true,
                devices: true,
                rooms: true,
                vaults: true,
                screens: true,
            },
            system_items,
            open_devices_with: OpenDevicesWith::App,
            open_folders_with: OpenFoldersWith::App,
            foreign_desktop_icons: ForeignDesktopIcons::Keep,
        }
    }

    pub(crate) fn from_json(text: &str) -> Result<DesktopConfig, ConfigError> {
        let config: DesktopConfig =
            serde_json::from_str(text).map_err(|error| ConfigError::Json(error.to_string()))?;
        if config.schema != CONFIG_SCHEMA {
            return Err(ConfigError::Schema(config.schema));
        }
        config.background.validate()?;
        Ok(config)
    }

    pub(crate) fn to_json(&self) -> Result<String, ConfigError> {
        serde_json::to_string_pretty(self).map_err(|error| ConfigError::Json(error.to_string()))
    }

    /// The background drawn: a stored `transparent` that the session cannot
    /// show falls back to the system background (the settings show why).
    pub(crate) fn effective_background(&self, transparent: &Result<(), String>) -> Background {
        if self.background.kind == BackgroundKind::Transparent && transparent.is_err() {
            Background::system()
        } else {
            self.background.clone()
        }
    }

    /// `file_manager` applies only while the S2 device tree is mounted (C4.6).
    pub(crate) fn effective_open_with(&self, device_tree_mounted: bool) -> OpenDevicesWith {
        if device_tree_mounted {
            self.open_devices_with
        } else {
            OpenDevicesWith::App
        }
    }
}

/// The options of one `se desktop config` call; `None` keeps the value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConfigChange {
    pub(crate) background: Option<Background>,
    pub(crate) desktop_files: Option<bool>,
    pub(crate) devices: Option<bool>,
    pub(crate) rooms: Option<bool>,
    pub(crate) vaults: Option<bool>,
    pub(crate) screens: Option<bool>,
    pub(crate) recycle_bin: Option<bool>,
    pub(crate) this_pc: Option<bool>,
    pub(crate) open_devices_with: Option<OpenDevicesWith>,
    pub(crate) open_folders_with: Option<OpenFoldersWith>,
    pub(crate) foreign_desktop_icons: Option<ForeignDesktopIcons>,
}

/// Facts of the session a change is checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigContext {
    pub(crate) platform: Platform,
    /// `mechanism::transparent_allowed` of the running session.
    pub(crate) transparent: Result<(), String>,
    /// `mounted` of S2 `DevicesStatus`.
    pub(crate) device_tree_mounted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfigError {
    Json(String),
    Schema(u32),
    InvalidBackground(String),
    TransparentRefused(String),
    DeviceTreeNotMounted,
    WindowsOnly(&'static str),
    LinuxOnly(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Json(error) => write!(formatter, "config.json ist ungültig: {error}"),
            ConfigError::Schema(schema) => {
                write!(formatter, "config.json hat das unbekannte Schema {schema}")
            }
            ConfigError::InvalidBackground(why) => {
                write!(formatter, "Ungültiger Hintergrund: {why}")
            }
            ConfigError::TransparentRefused(reason) => {
                write!(
                    formatter,
                    "Durchsichtiger Hintergrund nicht möglich: {reason}"
                )
            }
            ConfigError::DeviceTreeNotMounted => write!(
                formatter,
                "Geräte im Dateimanager öffnen braucht eingehängte \
                 „Verbundene Geräte als Ordner“"
            ),
            ConfigError::WindowsOnly(option) => {
                write!(formatter, "{option} gibt es nur unter Windows")
            }
            ConfigError::LinuxOnly(option) => write!(formatter, "{option} gibt es nur unter Linux"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Applies one `se desktop config` call; nothing changes on an error.
pub(crate) fn apply_change(
    config: &DesktopConfig,
    change: &ConfigChange,
    context: &ConfigContext,
) -> Result<DesktopConfig, ConfigError> {
    let mut next = config.clone();
    if let Some(background) = &change.background {
        background.validate()?;
        if background.kind == BackgroundKind::Transparent {
            if let Err(reason) = &context.transparent {
                return Err(ConfigError::TransparentRefused(reason.clone()));
            }
        }
        next.background = background.clone();
    }
    for (option, value) in [
        ("--recycle-bin", change.recycle_bin),
        ("--this-pc", change.this_pc),
    ] {
        if value.is_some() && context.platform != Platform::Windows {
            return Err(ConfigError::WindowsOnly(option));
        }
    }
    if change.foreign_desktop_icons.is_some() && context.platform != Platform::Linux {
        return Err(ConfigError::LinuxOnly("--foreign-desktop-icons"));
    }
    if change.open_devices_with == Some(OpenDevicesWith::FileManager)
        && !context.device_tree_mounted
    {
        return Err(ConfigError::DeviceTreeNotMounted);
    }
    let flags = [
        (&mut next.groups.desktop_files, change.desktop_files),
        (&mut next.groups.devices, change.devices),
        (&mut next.groups.rooms, change.rooms),
        (&mut next.groups.vaults, change.vaults),
        (&mut next.groups.screens, change.screens),
        (&mut next.system_items.recycle_bin, change.recycle_bin),
        (&mut next.system_items.this_pc, change.this_pc),
    ];
    for (flag, value) in flags {
        if let Some(value) = value {
            *flag = value;
        }
    }
    if let Some(open_with) = change.open_devices_with {
        next.open_devices_with = open_with;
    }
    if let Some(open_with) = change.open_folders_with {
        next.open_folders_with = open_with;
    }
    if let Some(icons) = change.foreign_desktop_icons {
        next.foreign_desktop_icons = icons;
    }
    Ok(next)
}
