//! The rights options of the compact connect and share flow (plan
//! 2026-10-31 §F, F3/F4): one preset and the single switches, the same on
//! `se share request accept`, `se share request policy auto-accept`,
//! `se share discoverable`, `se share identity`, `se share room create`,
//! `se share grants set` and `se share grants confirm`. The preset fills
//! the switches; every switch given changes it.
use clap::{Args, ValueEnum};
use clap_complete::ArgValueCandidates;
use serde_json::{json, Value};

use crate::share::access_grant::{AccountProfile, CapabilityDecision};
use crate::share::connection_rights::{ConnectionRights, FolderChoice, RightsPreset};
use crate::share::SharedRoot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum PresetArg {
    /// Keine Rechte: connected, nothing allowed
    None,
    /// Nur lesen: every Direct folder, read only
    ReadOnly,
    /// Dateien lesen und schreiben
    ReadWrite,
    /// Dateien und Codeausführung: read, write, commands, terminal
    FilesExec,
    /// Alles: files, commands, terminal, desktop view and control
    All,
}

impl From<PresetArg> for RightsPreset {
    fn from(value: PresetArg) -> Self {
        match value {
            PresetArg::None => Self::None,
            PresetArg::ReadOnly => Self::ReadOnly,
            PresetArg::ReadWrite => Self::ReadWrite,
            PresetArg::FilesExec => Self::FilesExec,
            PresetArg::All => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum DecisionArg {
    Off,
    Ask,
    Allow,
}

impl From<DecisionArg> for CapabilityDecision {
    fn from(value: DecisionArg) -> Self {
        match value {
            DecisionArg::Off => Self::Off,
            DecisionArg::Ask => Self::Ask,
            DecisionArg::Allow => Self::Allow,
        }
    }
}

#[derive(Args, Clone, Debug, Default)]
pub(super) struct RightsArgs {
    /// Rights preset (see `se share grants presets`); the options below change it
    #[arg(long, value_enum, value_name = "PRESET")]
    pub(super) preset: Option<PresetArg>,
    /// Only this Direct folder, by label or path (repeatable)
    #[arg(long = "folder", value_name = "FOLDER", conflicts_with_all = ["all_folders", "no_folders"],
        add = ArgValueCandidates::new(crate::cli::completions_access::folder_candidates))]
    pub(super) folders: Vec<String>,
    /// Every Direct folder, also folders exported later
    #[arg(long, conflicts_with = "no_folders")]
    pub(super) all_folders: bool,
    /// No folder at all
    #[arg(long)]
    pub(super) no_folders: bool,
    /// With --folder: also the exported saved connections (/Verbindungen)
    #[arg(long, requires = "folders")]
    pub(super) with_connections: bool,
    /// May write where a folder is read-write (create, change, delete, trash)
    #[arg(long, conflicts_with = "read_only")]
    pub(super) write: bool,
    /// May not write
    #[arg(long)]
    pub(super) read_only: bool,
    /// Programs and shell commands
    #[arg(long, value_enum, value_name = "off|ask|allow")]
    pub(super) commands: Option<DecisionArg>,
    /// A terminal with the login shell
    #[arg(long, value_enum, value_name = "off|ask|allow")]
    pub(super) terminal: Option<DecisionArg>,
    /// See the desktop
    #[arg(long, value_enum, value_name = "off|ask|allow")]
    pub(super) screen_view: Option<DecisionArg>,
    /// Control the desktop (includes programs, login, local services and the host's own Remote Desktop)
    #[arg(long, value_enum, value_name = "off|ask|allow")]
    pub(super) screen_control: Option<DecisionArg>,
    /// End commands, terminal and screen after 30m, 8h, 1d or 7d
    #[arg(long = "for", value_name = "DURATION", conflicts_with = "no_expiry")]
    pub(super) duration: Option<String>,
    /// No end
    #[arg(long)]
    pub(super) no_expiry: bool,
    /// Account commands and terminal run as: own, remote or account:NAME
    #[arg(long = "as", value_name = "own|remote|account:NAME",
        add = ArgValueCandidates::new(crate::cli::completions_access::account_candidates))]
    pub(super) account: Option<String>,
}

impl RightsArgs {
    /// Whether a preset or any switch was given.
    pub(super) fn given(&self) -> bool {
        self.preset.is_some() || self.switches_given()
    }

    /// Whether a switch other than the preset was given.
    pub(super) fn switches_given(&self) -> bool {
        !self.folders.is_empty()
            || self.all_folders
            || self.no_folders
            || self.write
            || self.read_only
            || self.commands.is_some()
            || self.terminal.is_some()
            || self.screen_view.is_some()
            || self.screen_control.is_some()
            || self.duration.is_some()
            || self.no_expiry
            || self.account.is_some()
    }

    /// Only --write or --read-only (the older `se share grants set`).
    pub(super) fn only_write(&self) -> bool {
        (self.write || self.read_only)
            && self.preset.is_none()
            && self.folders.is_empty()
            && !self.all_folders
            && !self.no_folders
            && self.commands.is_none()
            && self.terminal.is_none()
            && self.screen_view.is_none()
            && self.screen_control.is_none()
            && self.duration.is_none()
            && !self.no_expiry
            && self.account.is_none()
    }

    /// The chosen rights: the preset, else `base`, else „Keine Rechte“,
    /// changed by every switch given. `None` when nothing was given.
    /// `--folder` resolves a label or path among `roots`.
    pub(super) fn rights(
        &self,
        base: Option<ConnectionRights>,
        roots: &[SharedRoot],
        now: i64,
    ) -> Result<Option<ConnectionRights>, String> {
        if !self.given() {
            return Ok(None);
        }
        let mut rights = match self.preset {
            Some(preset) => RightsPreset::from(preset).rights(),
            None => base.unwrap_or_else(ConnectionRights::none),
        };
        if !self.folders.is_empty() {
            let paths = self
                .folders
                .iter()
                .map(|folder| resolve_folder(folder, roots))
                .collect::<Result<Vec<_>, _>>()?;
            rights.folders = FolderChoice::Selected {
                paths,
                connections: self.with_connections,
            };
        } else if self.all_folders {
            rights.folders = FolderChoice::All;
        } else if self.no_folders {
            rights.folders = FolderChoice::None;
        }
        if self.write || self.read_only {
            rights.write = self.write;
        }
        let switches = [
            (self.commands, &mut rights.commands),
            (self.terminal, &mut rights.terminal),
            (self.screen_view, &mut rights.screen_view),
            (self.screen_control, &mut rights.screen_control),
        ];
        for (chosen, decision) in switches {
            if let Some(chosen) = chosen {
                *decision = chosen.into();
            }
        }
        if let Some(duration) = &self.duration {
            rights.valid_until = Some(now.saturating_add(parse_duration(duration)?));
        } else if self.no_expiry {
            rights.valid_until = None;
        }
        if let Some(account) = &self.account {
            rights.account = parse_account(account)?;
        }
        rights.validate()?;
        Ok(Some(rights))
    }
}

/// A Direct folder by exact label (ignoring case) or exact path.
fn resolve_folder(folder: &str, roots: &[SharedRoot]) -> Result<String, String> {
    let folder = folder.trim();
    roots
        .iter()
        .find(|root| root.path == folder)
        .or_else(|| {
            let mut labelled = roots
                .iter()
                .filter(|root| root.label.eq_ignore_ascii_case(folder));
            let first = labelled.next();
            first.filter(|_| labelled.next().is_none())
        })
        .map(|root| root.path.clone())
        .ok_or_else(|| {
            format!(
                "--folder {folder}: no exported folder with this label or path; export it first with `se share export add`"
            )
        })
}

/// `30m`, `8h`, `1d`, `7d`, `1w` or combined `1d12h`, in seconds.
pub(super) fn parse_duration(text: &str) -> Result<i64, String> {
    let invalid =
        || format!("--for {text}: use minutes, hours, days or weeks such as 30m, 8h, 1d or 7d");
    let (mut total, mut number) = (0i64, None::<i64>);
    for character in text.trim().chars() {
        if let Some(digit) = character.to_digit(10) {
            let next = number.unwrap_or(0).checked_mul(10);
            number = Some(
                next.and_then(|n| n.checked_add(i64::from(digit)))
                    .ok_or_else(invalid)?,
            );
            continue;
        }
        let unit = match character {
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return Err(invalid()),
        };
        let part = number.take().ok_or_else(invalid)?.checked_mul(unit);
        total = part
            .and_then(|part| total.checked_add(part))
            .ok_or_else(invalid)?;
    }
    if number.is_some() || total <= 0 {
        return Err(invalid());
    }
    Ok(total)
}

fn parse_account(text: &str) -> Result<AccountProfile, String> {
    let account = match (text, text.strip_prefix("account:")) {
        ("own", _) => AccountProfile::Own,
        ("remote", _) => AccountProfile::Remote,
        (_, Some(name)) => AccountProfile::Account { name: name.into() },
        _ => return Err(format!("--as {text}: use own, remote or account:NAME")),
    };
    account.validate().map(|()| account)
}

/// The JSON form of `rights` every surface shows: the stored form plus the
/// matching preset key (or null) and the German summary.
pub(super) fn rights_value(rights: &ConnectionRights) -> Value {
    let mut value = serde_json::to_value(rights).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert(
            "preset".into(),
            json!(rights.preset().map(RightsPreset::key)),
        );
        map.insert("summary".into(), json!(rights.summary()));
    }
    value
}

/// Every preset with key, label, description and exact rights.
pub(super) fn presets_value() -> Value {
    Value::Array(
        RightsPreset::ALL
            .into_iter()
            .map(|preset| {
                json!({
                    "key": preset.key(),
                    "label": preset.label(),
                    "description": preset.description(),
                    "rights": rights_value(&preset.rights()),
                })
            })
            .collect(),
    )
}

/// One text line: `rights<TAB>LABEL<TAB>SUMMARY`.
pub(super) fn rights_text(label: &str, rights: &ConnectionRights) -> String {
    format!("{label}\t{}", rights.summary())
}

#[cfg(test)]
#[path = "rights_args_tests.rs"]
mod abgleich_task_tests;
