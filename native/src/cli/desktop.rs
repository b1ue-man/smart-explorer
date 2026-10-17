//! `se desktop`: se Desktop on Windows and Linux (DSK-M3, C5.8, C7 O1–O6,
//! O13, O14): status, enable, disable, remove, part, config and restart, each
//! with `--json`. Exit codes: 0 success, 1 error with text (also when traces
//! remain or a start failed), 2 call error (clap). The commands change only
//! this user's module, never the background service; the installers call
//! `enable`, `restart` and `remove`.
use clap::{Args, Subcommand, ValueEnum};

use crate::desktop::core::config::{
    Background, ConfigChange, ConfigContext, ForeignDesktopIcons, OpenDevicesWith,
};
use crate::desktop::core::state::Part;
use crate::desktop::os::control::{Control, ControlError, Outcome};
use crate::desktop::os::launch::with_control;
use crate::desktop::os::{platform, status_report, Session};

#[cfg(test)]
mod json_tests;
mod output;
#[cfg(test)]
mod tests;

const DESKTOP_HELP: &str = "\
se Desktop: a desktop surface with your devices, rooms and vaults on Windows
and Linux, run by the Smart Explorer app installed next to this se. Disable
and remove leave no trace; `se desktop status --json` lists every trace that
is still found as residue. se Desktop never starts the background service.

Examples:
  se desktop status --json
  se desktop enable
  se desktop enable --part surface
  se desktop part nav-pane off
  se desktop config --devices on --rooms off
  se desktop restart
  se desktop disable
  se desktop remove";

#[derive(Args)]
#[command(long_about = DESKTOP_HELP)]
pub(super) struct DesktopArgs {
    #[command(subcommand)]
    command: DesktopCommand,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum DesktopCommand {
    #[command(about = "Show state, parts, desktop layer, processes and remaining traces")]
    Status {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Add or enable se Desktop; without --part every available part")]
    Enable {
        #[arg(
            long = "part",
            value_enum,
            help = "Enable this part (repeatable); a part this session lacks is refused"
        )]
        parts: Vec<PartArg>,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Disable se Desktop and remove its traces; settings and arrangement stay")]
    Disable {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Remove se Desktop with every trace, its settings and its arrangement")]
    Remove {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Switch one part on or off")]
    Part {
        #[arg(
            value_enum,
            help = "surface, nav-pane or jump-list (the last two: Windows)"
        )]
        part: PartArg,
        #[arg(value_enum, help = "on or off")]
        state: Switch,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Show or change appearance, groups and behavior")]
    Config(ConfigArgs),
    #[command(about = "Restart se Desktop, also after a halt; the installers call it")]
    Restart {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum PartArg {
    Surface,
    NavPane,
    JumpList,
}

impl From<PartArg> for Part {
    fn from(part: PartArg) -> Part {
        match part {
            PartArg::Surface => Part::Surface,
            PartArg::NavPane => Part::NavPane,
            PartArg::JumpList => Part::JumpList,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Switch {
    On,
    Off,
}

impl Switch {
    fn on(self) -> bool {
        self == Switch::On
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OpenWith {
    App,
    FileManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ForeignIcons {
    Keep,
    TakeOver,
}

fn parse_background(value: &str) -> Result<Background, String> {
    Background::parse_cli(value).map_err(|error| error.to_string())
}

#[derive(Args, Debug, PartialEq, Eq)]
struct ConfigArgs {
    #[arg(
        long,
        value_parser = parse_background,
        help = "system, transparent, color:#RRGGBB or image:PATH"
    )]
    background: Option<Background>,
    #[arg(long, value_enum, help = "Show the files of the desktop folder")]
    desktop_files: Option<Switch>,
    #[arg(long, value_enum, help = "Show the devices")]
    devices: Option<Switch>,
    #[arg(long, value_enum, help = "Show the rooms")]
    rooms: Option<Switch>,
    #[arg(long, value_enum, help = "Show the vaults")]
    vaults: Option<Switch>,
    #[arg(long, value_enum, help = "Show the tile \"Alle Bildschirme\"")]
    screens: Option<Switch>,
    #[arg(long, value_enum, help = "Show \"Papierkorb\" (Windows)")]
    recycle_bin: Option<Switch>,
    #[arg(long, value_enum, help = "Show \"Dieser PC\" (Windows)")]
    this_pc: Option<Switch>,
    #[arg(
        long,
        value_enum,
        help = "Open devices in the se window or the file manager"
    )]
    open_devices_with: Option<OpenWith>,
    #[arg(
        long,
        value_enum,
        help = "GNOME desktop icons: keep or take over (Linux)"
    )]
    foreign_desktop_icons: Option<ForeignIcons>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

impl ConfigArgs {
    fn change(&self) -> ConfigChange {
        let flag = |value: Option<Switch>| value.map(Switch::on);
        ConfigChange {
            background: self.background.clone(),
            desktop_files: flag(self.desktop_files),
            devices: flag(self.devices),
            rooms: flag(self.rooms),
            vaults: flag(self.vaults),
            screens: flag(self.screens),
            recycle_bin: flag(self.recycle_bin),
            this_pc: flag(self.this_pc),
            open_devices_with: self.open_devices_with.map(|open| match open {
                OpenWith::App => OpenDevicesWith::App,
                OpenWith::FileManager => OpenDevicesWith::FileManager,
            }),
            foreign_desktop_icons: self.foreign_desktop_icons.map(|icons| match icons {
                ForeignIcons::Keep => ForeignDesktopIcons::Keep,
                ForeignIcons::TakeOver => ForeignDesktopIcons::TakeOver,
            }),
        }
    }
}

pub(super) fn run(args: DesktopArgs) -> Result<i32, String> {
    match args.command {
        DesktopCommand::Status { json } => {
            output::print_status(&status_report(), json)?;
            Ok(0)
        }
        DesktopCommand::Enable { parts, json } => {
            let parts: Vec<Part> = parts.into_iter().map(Part::from).collect();
            let requested = (!parts.is_empty()).then_some(parts);
            change(json, |control| control.enable(requested.as_deref()))
        }
        DesktopCommand::Disable { json } => change(json, |control| control.disable()),
        DesktopCommand::Remove { json } => change(json, |control| control.remove()),
        DesktopCommand::Part { part, state, json } => {
            change(json, |control| control.set_part(part.into(), state.on()))
        }
        DesktopCommand::Config(args) => config(&args),
        DesktopCommand::Restart { json } => restart(json),
    }
}

/// A transition, then the status as after `se desktop status`.
fn change(
    json: bool,
    act: impl FnOnce(&mut Control<'_>) -> Result<Outcome, ControlError>,
) -> Result<i32, String> {
    let outcome = with_control(act).map_err(|error| error.to_string())?;
    output::print_status(&status_report(), json)?;
    for error in &outcome.errors {
        eprintln!("se desktop: {error}");
    }
    Ok(if outcome.complete() { 0 } else { 1 })
}

fn config(args: &ConfigArgs) -> Result<i32, String> {
    let change = args.change();
    if change == ConfigChange::default() {
        output::print_config(status_report().config.as_ref(), args.json)?;
        return Ok(0);
    }
    let next = with_control(|control| {
        let stored = control.dir.read_config().ok().flatten();
        let session = Session::probe(stored.as_ref());
        let context = ConfigContext {
            platform: platform::PLATFORM,
            transparent: session.transparent,
            // The S2 device tree joins with DSK-M4 (`DevicesStatus`).
            device_tree_mounted: false,
        };
        control.configure(&change, &context)
    })
    .map_err(|error| error.to_string())?;
    output::print_config(Some(&next), args.json)?;
    Ok(0)
}

/// Not restarting an absent or disabled module is no error (the installers
/// call `restart` after every installation); a failed restart is.
fn restart(json: bool) -> Result<i32, String> {
    let outcome = with_control(|control| control.restart()).map_err(|error| error.to_string())?;
    output::print_restart(&outcome, json)?;
    if outcome.failed {
        if let Some(reason) = &outcome.reason {
            eprintln!("se desktop: {reason}");
        }
    }
    Ok(if outcome.failed { 1 } else { 0 })
}
