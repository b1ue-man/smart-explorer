//! `se screen` (SCR-M2, strand S4): devices whose screen this device can
//! open, their screen rights and facts (`probe`), a measurement with the test
//! picture (`bench`), and the sessions this device hosts (`sessions`,
//! `stop`). Each command has `--json`. Exit codes: 0 success, 1 error with
//! text (a refusal names its code), 2 call error (clap). Who may watch or
//! control this device is set with `se share grants access … --screen-view
//! --screen-control`; confirmations are answered with `se share exec
//! approvals|approve|deny`, never through MCP.
use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::share::{PeerOpenTarget, ShareProfiles};

mod bench;
mod output;
mod sessions;
#[cfg(test)]
mod tests;

const SCREEN_HELP: &str = "\
Watch the screen of another device and see who watches this one. A device
shows its screen only to devices it allows „Bildschirm ansehen“ (off, ask or
allow) with `se share grants access DEVICE --screen-view ask`; on ask its user
confirms every session. A device without the desktop app shows only the test
picture. This version measures the path with the test picture; viewing a
desktop follows with the desktop viewer.

Devices are named like `se exec` does: share://direct/<contact-id>,
share://room/<room-id>/<device-id>, a name, an id or a fingerprint prefix.

Examples:
  se screen devices --probe --json
  se screen probe Laptop --json
  se screen bench Laptop --seconds 20 --json
  se screen bench --local --size 1920x1080
  se screen sessions --json
  se screen stop --all";

#[derive(Args)]
#[command(long_about = SCREEN_HELP)]
pub(super) struct ScreenArgs {
    #[command(subcommand)]
    command: ScreenCommand,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum ScreenCommand {
    #[command(about = "List the devices whose screen this device can open")]
    Devices {
        /// Only the members of this room (profile id, room id or name)
        #[arg(long, value_name = "ROOM", add = ArgValueCandidates::new(super::completions_access::room_candidates))]
        room: Option<String>,
        /// Ask each online device for its rights and facts
        #[arg(long)]
        probe: bool,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Ask a device for screen rights, platform, codecs, monitors and limits")]
    Probe {
        #[arg(help = "Device: share:// endpoint, name, id or fingerprint prefix", add = ArgValueCandidates::new(super::completions_screen::device_candidates))]
        device: String,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Measure the screen path with the test picture")]
    Bench(bench::BenchArgs),
    #[command(about = "Show the screen sessions this device hosts")]
    Sessions {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "End screen sessions this device hosts")]
    Stop {
        #[arg(required_unless_present = "all", conflicts_with = "all", help = "Session id from `se screen sessions`", add = ArgValueCandidates::new(super::completions_screen::session_candidates))]
        session: Option<String>,
        /// Every session
        #[arg(long)]
        all: bool,
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
}

pub(super) fn run(args: ScreenArgs) -> Result<i32, String> {
    match args.command {
        ScreenCommand::Devices { room, probe, json } => devices(room.as_deref(), probe, json),
        ScreenCommand::Probe { device, json } => {
            let target = select_device(&device)?.target;
            let report = crate::daemon::screen_probe(target)?;
            output::print(json, &report, || output::probe_text(&report));
            Ok(0)
        }
        ScreenCommand::Bench(args) => bench::run(args),
        ScreenCommand::Sessions { json } => sessions::list(json),
        ScreenCommand::Stop { session, all, json } => sessions::stop(session, all, json),
    }
}

/// One device whose screen this device can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceChoice {
    pub(super) target: PeerOpenTarget,
    pub(super) endpoint: String,
    pub(super) name: String,
    pub(super) device_id: String,
    /// `direct` or `room`.
    pub(super) relation: String,
    pub(super) room: Option<String>,
    pub(super) online: bool,
    selectors: Vec<String>,
}

/// Direct contacts and the members of joined rooms (the targets of `se
/// exec`), in that order.
pub(super) fn device_choices(profiles: &ShareProfiles, now: i64) -> Vec<DeviceChoice> {
    let current = |presence: Option<&crate::share::PeerPresence>| {
        presence.is_some_and(|presence| presence.is_current_at(now))
    };
    let direct = profiles.direct_contacts.iter().map(|contact| {
        let endpoint = format!("share://direct/{}", contact.id);
        let device_id = contact.remote_device_id.clone().unwrap_or_default();
        DeviceChoice {
            target: PeerOpenTarget::Direct {
                contact_id: contact.id.clone(),
            },
            selectors: vec![
                endpoint.clone(),
                contact.id.clone(),
                contact.display_name.clone(),
                device_id.clone(),
                contact.expected_fingerprint.clone(),
            ],
            endpoint,
            name: contact.display_name.clone(),
            device_id,
            relation: "direct".into(),
            room: None,
            online: current(contact.presence.as_ref())
                && contact.access_state == crate::share::DirectAccessState::Accepted,
        }
    });
    let rooms = profiles.rooms.iter().filter(|room| room.auto_join);
    let members = rooms.flat_map(|room| {
        room.members
            .iter()
            .filter(|member| !member.blocked)
            .map(move |member| {
                let endpoint = format!("share://room/{}/{}", room.room_id, member.device_id);
                DeviceChoice {
                    target: PeerOpenTarget::RoomDevice {
                        room_id: room.room_id.clone(),
                        device_id: member.device_id.clone(),
                    },
                    selectors: vec![
                        endpoint.clone(),
                        member.device_id.clone(),
                        member.device_name.clone(),
                        member.fingerprint.clone(),
                        format!("{}/{}", room.name, member.device_name),
                    ],
                    endpoint,
                    name: member.device_name.clone(),
                    device_id: member.device_id.clone(),
                    relation: "room".into(),
                    room: Some(room.name.clone()),
                    online: current(member.presence.as_ref()),
                }
            })
    });
    direct.chain(members).collect()
}

/// Whether `selector` names `candidate`: equal, or a prefix of at least
/// four characters, ignoring case (as `se exec`).
fn selector_matches(selector: &str, candidate: &str) -> bool {
    !candidate.is_empty()
        && (candidate.eq_ignore_ascii_case(selector)
            || (selector.len() >= 4
                && candidate
                    .to_ascii_lowercase()
                    .starts_with(&selector.to_ascii_lowercase())))
}

/// The one device `selector` names among `choices`.
pub(super) fn select_from(
    choices: &[DeviceChoice],
    selector: &str,
) -> Result<DeviceChoice, String> {
    let selector = selector.trim();
    if let Some((target, path)) = PeerOpenTarget::from_endpoint(selector) {
        if path != "/" {
            return Err("a screen device takes no path".into());
        }
        if let Some(choice) = choices.iter().find(|choice| choice.target == target) {
            return Ok(choice.clone());
        }
        return Err(format!("device not found: {selector}"));
    }
    let matches: Vec<&DeviceChoice> = choices
        .iter()
        .filter(|choice| {
            choice
                .selectors
                .iter()
                .any(|candidate| selector_matches(selector, candidate))
        })
        .collect();
    match matches.as_slice() {
        [choice] => Ok((*choice).clone()),
        [] => Err(format!(
            "device not found: {selector}; see `se screen devices`"
        )),
        _ => {
            let listed: Vec<String> = matches
                .iter()
                .map(|choice| format!("  {}\t{}", choice.endpoint, choice.name))
                .collect();
            Err(format!(
                "device selector is ambiguous:\n{}",
                listed.join("\n")
            ))
        }
    }
}

fn select_device(selector: &str) -> Result<DeviceChoice, String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let choices = device_choices(&snapshot.profiles, crate::share::core_now_secs());
    select_from(&choices, selector)
}

fn devices(room: Option<&str>, probe: bool, json: bool) -> Result<i32, String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let profiles = &snapshot.profiles;
    let mut choices = device_choices(profiles, crate::share::core_now_secs());
    if let Some(room) = room {
        let found = profiles
            .rooms
            .iter()
            .find(|candidate| {
                candidate.id == room || candidate.room_id == room || candidate.name == room
            })
            .ok_or_else(|| format!("room not found: {room}"))?;
        let in_room = |choice: &DeviceChoice| match &choice.target {
            PeerOpenTarget::RoomDevice { room_id, .. } => *room_id == found.room_id,
            PeerOpenTarget::Direct { .. } => false,
        };
        choices.retain(in_room);
    }
    let probes: Vec<Option<Result<crate::daemon::ScreenProbeReport, String>>> = choices
        .iter()
        .map(|choice| {
            (probe && choice.online).then(|| crate::daemon::screen_probe(choice.target.clone()))
        })
        .collect();
    if json {
        println!("{}", output::devices_json(&choices, &probes));
    } else {
        print!("{}", output::devices_text(&choices, &probes));
    }
    Ok(0)
}
