//! `se screen view DEVICE [--source QUELLE] [--control] [--fullscreen] [--json]`
//! and `se screen grid [--room RAUM] [--json]` (SCR-M7): open the desktop
//! viewer window of the GUI program (`--screen-view <ziel-json>`,
//! `--screen-grid [--room <id>]`, V5) as a process of its own and return.
//! The window asks the device and its user itself. Output: `launched\t<pid>`
//! or, with `--json`, `{"launched":true,"pid":<n>}`. Without the desktop app
//! next to this `se` the command fails with „Desktop-App nicht installiert“.
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::{json, Value};

use super::DeviceChoice;
use crate::app::start_request::StartRequest;
use crate::screen::core::window_list::SourceId;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct ViewArgs {
    #[arg(
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: String,
    /// Source: monitor:<id> or window:<id> (default: the whole screen)
    #[arg(long, value_name = "QUELLE")]
    source: Option<String>,
    /// Start with control („Bildschirm steuern“)
    #[arg(long)]
    control: bool,
    /// Open in full screen
    #[arg(long)]
    fullscreen: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct GridArgs {
    /// Only the members of this room (profile id, room id or name)
    #[arg(
        long,
        value_name = "RAUM",
        add = ArgValueCandidates::new(crate::cli::completions_access::room_candidates)
    )]
    room: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// The target JSON of `--screen-view`: `relation`, `relation_id`,
/// `device_id`, `source` (text or null), `control`, and `fullscreen` only
/// when asked for (the start parser allows one value after the switch).
pub(super) fn target_json(
    choice: &DeviceChoice,
    source: Option<&str>,
    control: bool,
    fullscreen: bool,
) -> Result<Value, String> {
    if choice.device_id.is_empty() {
        return Err(format!(
            "{} has no device id yet; it appears after the first connection",
            choice.name
        ));
    }
    let source = source
        .map(|text| {
            SourceId::parse(text)
                .map(|source| source.to_string())
                .map_err(|error| format!("invalid source: {error}"))
        })
        .transpose()?;
    let relation_id = match &choice.target {
        crate::share::PeerOpenTarget::Direct { contact_id } => contact_id.clone(),
        crate::share::PeerOpenTarget::RoomDevice { room_id, .. } => room_id.clone(),
    };
    let mut target = json!({
        "relation": choice.relation,
        "relation_id": relation_id,
        "device_id": choice.device_id,
        "source": source,
        "control": control,
    });
    if fullscreen {
        target["fullscreen"] = Value::Bool(true);
    }
    Ok(target)
}

/// Starts the GUI program with `request`; the process ID.
fn launch(request: &StartRequest) -> Result<u32, String> {
    let gui = crate::desktop::os::launch::gui_binary()
        .map_err(|_| "Desktop-App nicht installiert".to_string())?;
    let args = crate::desktop::os::open_target::window_args(request);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = crate::desktop::os::platform::process::spawn_detached(&gui, &args)
        .map_err(|error| format!("{}: {error}", gui.display()))?;
    let pid = child.id();
    crate::desktop::os::launch::reap(child);
    Ok(pid)
}

fn report(json: bool, pid: u32) {
    super::output::print(json, &json!({"launched": true, "pid": pid}), || {
        format!("launched\t{pid}\n")
    });
}

pub(super) fn run_view(args: ViewArgs) -> Result<i32, String> {
    let choice = super::select_device(&args.device)?;
    let target = target_json(
        &choice,
        args.source.as_deref(),
        args.control,
        args.fullscreen,
    )?;
    let request = StartRequest::ScreenView {
        target_json: target.to_string(),
    };
    report(args.json, launch(&request)?);
    Ok(0)
}

/// The local profile ID of `room` (profile ID, room ID or name).
fn room_profile_id(room: &str) -> Result<String, String> {
    let Ok(snapshot) = crate::daemon::share_worker_snapshot() else {
        // The window reads the profiles itself and says what it misses.
        return Ok(room.to_string());
    };
    snapshot
        .profiles
        .rooms
        .iter()
        .find(|candidate| {
            candidate.id == room || candidate.room_id == room || candidate.name == room
        })
        .map(|found| found.id.clone())
        .ok_or_else(|| format!("room not found: {room}"))
}

pub(super) fn run_grid(args: GridArgs) -> Result<i32, String> {
    let room = args.room.as_deref().map(room_profile_id).transpose()?;
    let request = StartRequest::ScreenGrid { room };
    report(args.json, launch(&request)?);
    Ok(0)
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
