//! `se drive set <SELEKTOR> --auto-remount on|off [--json]` (DRV-M3, strand
//! S2 C7 of plan 2026-10-17) and the completion of drive selectors.
//!
//! Text: `drive\t<id>\tauto-remount=<on|off>`. JSON: the `MountSnapshot` of
//! the drive, as `mount`, `unmount` and `retry` print it; the option appears
//! as `config.auto_remount` only when it is on. Exit code 0; failures leave
//! like every `se drive` command. Through MCP the command runs with the `se`
//! tool.
use clap::{Args, ValueEnum};
use clap_complete::ArgValueCandidates;

use crate::mount::MountSnapshot;

#[derive(Args)]
pub(super) struct SetArgs {
    #[arg(
        help = "Mount id from `se drive list`, or a mounted letter such as M:",
        add = ArgValueCandidates::new(selector_candidates)
    )]
    selector: String,
    #[arg(
        long,
        value_enum,
        value_name = "ON|OFF",
        help = "Mount the drive again at every login and after a background-worker restart"
    )]
    auto_remount: Switch,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Switch {
    On,
    Off,
}

pub(super) fn set(args: SetArgs) -> Result<i32, String> {
    let id = super::resolve_selector(&args.selector)?;
    let snapshot = crate::daemon::set_mount_options(id, args.auto_remount == Switch::On)?;
    println!("{}", render(&snapshot, args.json)?);
    Ok(0)
}

fn render(snapshot: &MountSnapshot, json: bool) -> Result<String, String> {
    if json {
        return serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string());
    }
    Ok(format!(
        "drive\t{}\tauto-remount={}",
        snapshot.config.id,
        on_off(snapshot.config.auto_remount)
    ))
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// Completion of `se drive set|unmount|retry`: the registered drives, read
/// from the registry without starting the background worker.
pub(super) fn selector_candidates() -> Vec<clap_complete::CompletionCandidate> {
    crate::daemon::registered_for_completion()
        .iter()
        .map(|config| {
            let help = format!(
                "{} · auto-remount {}",
                config.label,
                on_off(config.auto_remount)
            )
            .replace(['\t', '\r', '\n'], " ");
            clap_complete::CompletionCandidate::new(config.id.as_str())
                .help(Some(clap::builder::StyledStr::from(help)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{render, SetArgs, Switch};
    use crate::mount::{
        BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery,
        MountSnapshot, MountSource, MountStatus,
    };
    use clap::Parser;

    #[derive(Parser)]
    struct SetCommand {
        #[command(flatten)]
        set: SetArgs,
    }

    #[derive(Parser)]
    struct DriveCli {
        #[command(flatten)]
        drive: super::super::DriveArgs,
    }

    fn snapshot(auto_remount: bool) -> MountSnapshot {
        let config = MountConfig::new(
            MountId::parse("0123456789abcdef").unwrap(),
            MountSource::SavedRemote {
                account: "sftp:prod".into(),
                root: BackendRoot::parse("/srv").unwrap(),
            },
            DriveSelection::Automatic,
            MountMode::ReadOnly,
            "Projekte",
        )
        .unwrap()
        .with_auto_remount(auto_remount);
        MountSnapshot {
            config,
            status: MountStatus::Mounted {
                drive: DriveLetter::parse('M').unwrap(),
            },
            recovery: MountRecovery::Clean,
            recovery_required_compat: false,
        }
    }

    #[test]
    fn drive_task_cli_set_text_and_json() {
        // Parsing: selector, a required on|off value and --json.
        let on = SetCommand::try_parse_from(["set", "0123abcd", "--auto-remount", "on", "--json"])
            .unwrap();
        assert_eq!(on.set.selector, "0123abcd");
        assert_eq!(on.set.auto_remount, Switch::On);
        assert!(on.set.json);
        let off = SetCommand::try_parse_from(["set", "M:", "--auto-remount", "off"]).unwrap();
        assert_eq!(off.set.auto_remount, Switch::Off);
        assert!(!off.set.json);
        assert!(SetCommand::try_parse_from(["set", "M:"]).is_err());
        assert!(SetCommand::try_parse_from(["set", "M:", "--auto-remount", "yes"]).is_err());
        // `se drive set` and `se drive mount --auto-remount` exist beside the
        // commands of v0.5.182.
        let parsed =
            DriveCli::try_parse_from(["drive", "set", "M:", "--auto-remount", "on"]).unwrap();
        assert!(matches!(
            parsed.drive.command,
            super::super::DriveCommand::Set(_)
        ));
        let mount =
            DriveCli::try_parse_from(["drive", "mount", "@prod:/srv", "--auto-remount"]).unwrap();
        assert!(matches!(parsed_mount_auto_remount(mount), Some(true)));
        let earlier: [&[&str]; 3] = [
            &["drive", "list"],
            &["drive", "unmount", "M:"],
            &["drive", "retry", "M:"],
        ];
        for command in earlier {
            assert!(DriveCli::try_parse_from(command).is_ok());
        }

        // Text output: one tab-separated line.
        assert_eq!(
            render(&snapshot(true), false).unwrap(),
            "drive\t0123456789abcdef\tauto-remount=on"
        );
        assert_eq!(
            render(&snapshot(false), false).unwrap(),
            "drive\t0123456789abcdef\tauto-remount=off"
        );
        // JSON output: the MountSnapshot of v0.5.182 plus the option, which
        // appears only when it is on.
        let on: serde_json::Value =
            serde_json::from_str(&render(&snapshot(true), true).unwrap()).unwrap();
        assert_eq!(on["config"]["auto_remount"], true);
        assert_eq!(on["status"]["Mounted"]["drive"], "M");
        assert_eq!(on["recovery"], "Clean");
        assert_eq!(on["recovery_required"], false);
        assert!(on["config"].get("location").is_none());
        let off: serde_json::Value =
            serde_json::from_str(&render(&snapshot(false), true).unwrap()).unwrap();
        assert!(off["config"].get("auto_remount").is_none());
        // A chosen folder is listed as `config.location` (`list --json`).
        let mut placed = snapshot(true);
        let folder = std::env::temp_dir().join("SmartExplorer").join("Projekte");
        placed.config = placed
            .config
            .with_location_directory(Some(folder.clone()))
            .unwrap();
        let placed: serde_json::Value =
            serde_json::from_str(&render(&placed, true).unwrap()).unwrap();
        assert_eq!(
            placed["config"]["location"]["Directory"]["path"],
            folder.to_str().unwrap()
        );
    }

    fn parsed_mount_auto_remount(command: DriveCli) -> Option<bool> {
        match command.drive.command {
            super::super::DriveCommand::Mount(args) => Some(args.auto_remount),
            _ => None,
        }
    }
}
