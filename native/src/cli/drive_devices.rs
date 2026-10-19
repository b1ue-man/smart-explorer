//! `se drive devices status|watch|on|off` (DRV-M6, contract V4): the device
//! tree „Verbundene Geräte als Ordner“.
//!
//! Text of `status` and `watch`: one line
//! `devices\t<on|off>\t<state>\t<mounted|unmounted>\t<root path|->` and per
//! device `device\t<key>\t<name>\t<direct|room:<room>>\t<online|offline>\t
//! <path|->`. JSON: the status object of the worker. `watch` prints exactly
//! one answer, after a change since `--since` or after 30 s at the latest.
//! Exit codes: 0 done, 1 the worker refused (for example the tree is
//! already on), 2 a wrong option for this platform. Through MCP the
//! commands run with the `se` tool.
use std::path::PathBuf;

use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::daemon::DevicesStatus;

use super::drive_print::clean;

#[derive(Args)]
pub(super) struct DevicesArgs {
    #[command(subcommand)]
    command: DevicesCommand,
}

#[derive(Subcommand)]
enum DevicesCommand {
    #[command(about = "Show whether the device tree is on and which devices it holds")]
    Status(JsonArgs),
    #[command(about = "Wait for a change of the device tree (at most 30 seconds) and show it")]
    Watch(WatchArgs),
    #[command(about = "Switch the device tree on: every paired device appears as a folder")]
    On(OnArgs),
    #[command(about = "Switch the device tree off; unsaved changes of devices are kept")]
    Off(JsonArgs),
}

#[derive(Args)]
struct JsonArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct WatchArgs {
    #[arg(
        long,
        default_value_t = 0,
        value_name = "N",
        help = "Revision of the last answer; the answer comes when it differs"
    )]
    since: u64,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct OnArgs {
    #[arg(
        long,
        value_name = "auto|X",
        help = "Drive letter (Windows only)",
        add = ArgValueCandidates::new(letter_candidates)
    )]
    letter: Option<String>,
    #[arg(long, value_name = "PATH", help = "Mount folder (Linux only)")]
    mount_point: Option<PathBuf>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: DevicesArgs) -> Result<i32, String> {
    match args.command {
        DevicesCommand::Status(args) => print_status(&crate::daemon::devices_status()?, args.json),
        DevicesCommand::Watch(args) => {
            print_status(&crate::daemon::devices_watch(args.since)?, args.json)
        }
        DevicesCommand::On(args) => switch_on(args),
        DevicesCommand::Off(args) => print_status(&crate::daemon::devices_disable()?, args.json),
    }
}

fn switch_on(args: OnArgs) -> Result<i32, String> {
    if let Some(refusal) = platform_refusal(args.letter.is_some(), args.mount_point.is_some()) {
        eprintln!("se: {refusal}");
        return Ok(2);
    }
    let folder = args
        .mount_point
        .as_deref()
        .map(super::drive_options::absolute_folder)
        .transpose()?
        .map(|path| path.to_string_lossy().into_owned());
    let status = crate::daemon::devices_enable(args.letter, folder)?;
    print_status(&status, args.json)
}

/// The option that does not exist on this platform.
fn platform_refusal(letter: bool, mount_point: bool) -> Option<&'static str> {
    if cfg!(windows) {
        mount_point.then_some("Einhängeorte gibt es nur unter Linux; nutze --letter")
    } else {
        letter.then_some("Laufwerksbuchstaben gibt es nur unter Windows; nutze --mount-point")
    }
}

/// `auto` and `D` to `Z` under Windows; Linux has no letters.
fn letter_candidates() -> Vec<clap_complete::CompletionCandidate> {
    if !cfg!(windows) {
        return Vec::new();
    }
    std::iter::once("auto".to_string())
        .chain(('D'..='Z').map(|letter| letter.to_string()))
        .map(clap_complete::CompletionCandidate::new)
        .collect()
}

pub(super) fn print_status(status: &DevicesStatus, json: bool) -> Result<i32, String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(status).map_err(|error| error.to_string())?
        );
    } else {
        for line in status_lines(status) {
            println!("{line}");
        }
    }
    Ok(0)
}

/// The text form of an answer.
fn status_lines(status: &DevicesStatus) -> Vec<String> {
    let mut lines = vec![format!(
        "devices\t{}\t{}\t{}\t{}",
        if status.enabled { "on" } else { "off" },
        status.state,
        if status.mounted {
            "mounted"
        } else {
            "unmounted"
        },
        status
            .root_path
            .as_deref()
            .map_or_else(|| "-".to_string(), clean),
    )];
    for device in &status.devices {
        let kind = match (&device.room, device.kind.as_str()) {
            (Some(room), _) => format!("room:{}", clean(room)),
            (None, kind) => kind.to_string(),
        };
        lines.push(format!(
            "device\t{}\t{}\t{}\t{}\t{}",
            clean(&device.key),
            clean(&device.name),
            kind,
            if device.online { "online" } else { "offline" },
            device
                .path
                .as_deref()
                .map_or_else(|| "-".to_string(), clean),
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::DevicesStatus;
    use serde_json::json;

    fn status() -> DevicesStatus {
        serde_json::from_value(json!({
            "enabled": true,
            "state": "mounted",
            "mounted": true,
            "root_path": "/home/u/SmartExplorer/Geräte",
            "mount_point": "/home/u/SmartExplorer/Geräte",
            "revision": 3,
            "devices": [
                {"key": "direct~a", "name": "Lap\ttop", "kind": "direct", "online": true,
                 "path": "/home/u/SmartExplorer/Geräte/Laptop"},
                {"key": "room~r~d", "name": "Raum", "kind": "room", "room": "Te\nam",
                 "online": false}
            ],
            "pending": 1
        }))
        .unwrap()
    }

    #[test]
    fn drive_task_cli_devices_shapes() {
        let lines = status_lines(&status());
        assert_eq!(
            lines,
            [
                "devices\ton\tmounted\tmounted\t/home/u/SmartExplorer/Geräte",
                "device\tdirect~a\tLap top\tdirect\tonline\t/home/u/SmartExplorer/Geräte/Laptop",
                "device\troom~r~d\tRaum\troom:Te am\toffline\t-",
            ]
        );
        // Off: the same two kinds of lines with their dashes.
        let off: DevicesStatus = serde_json::from_value(json!({
            "enabled": false, "state": "off", "mounted": false, "revision": 1,
            "devices": []
        }))
        .unwrap();
        assert_eq!(status_lines(&off), ["devices\toff\toff\tunmounted\t-"]);
        // JSON is the object of the worker: absent values stay absent.
        let value = serde_json::to_value(&off).unwrap();
        assert!(value.get("root_path").is_none() && value.get("letter").is_none());
        assert_eq!(value["pending"], 0);
        // Each platform refuses the option of the other, with exit code 2.
        assert!(platform_refusal(false, false).is_none());
        if cfg!(windows) {
            assert!(platform_refusal(false, true).is_some_and(|text| text.contains("--letter")));
            assert!(platform_refusal(true, false).is_none());
        } else {
            assert!(
                platform_refusal(true, false).is_some_and(|text| text.contains("--mount-point"))
            );
            assert!(platform_refusal(false, true).is_none());
        }
    }
}
