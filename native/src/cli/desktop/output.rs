//! Output of `se desktop`: tab-separated text and JSON (DSK-M3 contract).
//! The JSON of every changing command is the status object, so
//! `se desktop disable --json` and `se desktop status --json` read alike.
use serde_json::{json, Value};

use crate::desktop::core::config::{
    BackgroundKind, DesktopConfig, ForeignDesktopIcons, OpenDevicesWith,
};
use crate::desktop::core::mechanism::Mechanism;
use crate::desktop::os::control::RestartOutcome;
use crate::desktop::os::status::StatusReport;

fn print_json(value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{text}");
    Ok(())
}

pub(super) fn print_status(report: &StatusReport, json: bool) -> Result<(), String> {
    if json {
        print_json(&report.json())
    } else {
        print!("{}", status_text(report));
        Ok(())
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// `state …`, one `part …` line per part, then `mechanism`, under GNOME on
/// Wayland `extension_state`, `surfaces`, `share_state`, `autostart`,
/// `halted`, `foreign_desktop` and `residue`.
pub(super) fn status_text(report: &StatusReport) -> String {
    let mut lines = vec![format!("state\t{}", report.state.key())];
    for part in &report.parts {
        let activity = match (&part.unavailable, part.active) {
            (Some(code), _) => format!("unavailable:{code}"),
            (None, true) => "active".into(),
            (None, false) => "inactive".into(),
        };
        lines.push(format!(
            "part\t{}\t{}\t{activity}",
            part.part.key(),
            on_off(part.enabled)
        ));
    }
    let mechanism = match &report.mechanism {
        Mechanism::Unavailable(reason) => format!("none:{}", reason.code()),
        mechanism => mechanism.key().to_string(),
    };
    lines.push(format!("mechanism\t{mechanism}"));
    if let Some(state) = report.extension_state {
        lines.push(format!("extension_state\t{state}"));
    }
    lines.push(format!(
        "surfaces\t{}/{}",
        report.surfaces.len(),
        report.monitors
    ));
    let share = if report.share_live {
        "live".to_string()
    } else {
        format!(
            "profiles:{}",
            report.share_reason.as_deref().unwrap_or_default()
        )
    };
    lines.push(format!("share_state\t{share}"));
    lines.push(format!("autostart\t{}", report.autostart.key()));
    let halted = report.halted.as_ref().map_or_else(
        || "no".to_string(),
        |halt| format!("crash_loop:{}", halt.version),
    );
    lines.push(format!("halted\t{halted}"));
    let foreign = report.foreign_desktop.as_ref().map_or_else(
        || "none".to_string(),
        |(kind, id, state)| format!("{kind}:{id}:{state}"),
    );
    lines.push(format!("foreign_desktop\t{foreign}"));
    lines.push(format!("residue\t{}", report.residue.len()));
    if let Some(error) = &report.state_error {
        lines.push(format!("state_error\t{error}"));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// `{"config": …}`; `null` while se Desktop is not added.
pub(super) fn config_json(config: Option<&DesktopConfig>) -> Value {
    json!({ "config": serde_json::to_value(config).unwrap_or(Value::Null) })
}

pub(super) fn config_text(config: Option<&DesktopConfig>) -> String {
    let Some(config) = config else {
        return "config\tnone\n".into();
    };
    let background = match (config.background.kind, &config.background.value) {
        (BackgroundKind::System, _) => "system".to_string(),
        (BackgroundKind::Transparent, _) => "transparent".to_string(),
        (BackgroundKind::Color, value) => format!("color:{}", value.as_deref().unwrap_or("")),
        (BackgroundKind::Image, value) => format!("image:{}", value.as_deref().unwrap_or("")),
    };
    let groups = &config.groups;
    let lines = [
        format!("background\t{background}"),
        format!("desktop_files\t{}", on_off(groups.desktop_files)),
        format!("devices\t{}", on_off(groups.devices)),
        format!("rooms\t{}", on_off(groups.rooms)),
        format!("vaults\t{}", on_off(groups.vaults)),
        format!("screens\t{}", on_off(groups.screens)),
        format!("recycle_bin\t{}", on_off(config.system_items.recycle_bin)),
        format!("this_pc\t{}", on_off(config.system_items.this_pc)),
        format!(
            "open_devices_with\t{}",
            match config.open_devices_with {
                OpenDevicesWith::App => "app",
                OpenDevicesWith::FileManager => "file-manager",
            }
        ),
        format!(
            "foreign_desktop_icons\t{}",
            match config.foreign_desktop_icons {
                ForeignDesktopIcons::Keep => "keep",
                ForeignDesktopIcons::TakeOver => "take-over",
            }
        ),
    ];
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

pub(super) fn print_config(config: Option<&DesktopConfig>, json: bool) -> Result<(), String> {
    if json {
        print_json(&config_json(config))
    } else {
        print!("{}", config_text(config));
        Ok(())
    }
}

/// `{"restarted": bool, "state", "reason"}` (DSK-M3 contract).
pub(super) fn restart_json(outcome: &RestartOutcome) -> Value {
    json!({
        "restarted": outcome.restarted,
        "state": outcome.state.key(),
        "reason": outcome.reason,
    })
}

pub(super) fn print_restart(outcome: &RestartOutcome, json: bool) -> Result<(), String> {
    if json {
        return print_json(&restart_json(outcome));
    }
    println!(
        "restarted\t{}",
        if outcome.restarted { "yes" } else { "no" }
    );
    println!("state\t{}", outcome.state.key());
    if let Some(reason) = &outcome.reason {
        println!("reason\t{reason}");
    }
    Ok(())
}
