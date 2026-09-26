//! `se share discoverable`: make this device or a room findable by name and
//! PIN for a limited time; list and stop the running offers.
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};

use super::discoverable_input::{
    duration_secs, pin_source, read_pin, resolve_room, select_offers, trivial_pin, validate_name,
};
use super::discoverable_output::{
    clean, local_time, offer_text, offer_value, stop_reason_text, target_text,
};
use crate::daemon::{ShareCommandReply, ShareWorkerSnapshot};
use crate::share::{
    DiscoveryCommand, DiscoveryPin, DiscoveryPublishTarget, OwnDiscoveryOffer, ShareCmd,
    ShareProfiles,
};

/// How long a new offer is awaited for the Share server's confirmation.
const PUBLISH_CONFIRMATION_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const INACTIVE_SHARE: &str = concat!(
    "Share is not active on this device: configure it with ",
    "`se share configure --server <endpoint>`, or start a stopped worker with ",
    "`se share worker refresh`"
);

const DISCOVERABLE_HELP: &str = "\
Publishes this device, or a room with --room, on the Share server under a name
for a limited time (default 5 minutes). Another device finds the name in its
discovery list and pairs by entering the same PIN; the Share server only relays
the pairing and never learns the PIN. Without a subcommand this starts an offer:

  se share discoverable --minutes 5 --pin 1454
  printf '%s\\n' 1454 | se share discoverable --pin-stdin
  se share discoverable --room Team --name \"Team laptop\"   (asks for the PIN)

--pin is visible in the process list; --pin-stdin or the hidden prompt keep the
PIN out of it. `list` shows the running offers with their end time; `stop` ends
one (by id or unique id prefix) or all of them.";

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true, long_about = DISCOVERABLE_HELP)]
pub(super) struct DiscoverableArgs {
    #[command(subcommand)]
    command: Option<DiscoverableCommand>,
    #[command(flatten)]
    publish: PublishArgs,
}

#[derive(Subcommand)]
enum DiscoverableCommand {
    #[command(about = "Show this device's running discoverable offers")]
    List(ListArgs),
    #[command(about = "Stop one discoverable offer or all of them")]
    Stop(StopArgs),
}

#[derive(Args)]
struct PublishArgs {
    #[arg(
        long,
        value_name = "ROOM",
        help = "Make this room discoverable instead of this device (room id or name)"
    )]
    room: Option<String>,
    #[arg(
        long,
        value_name = "NAME",
        help = "Name the other device sees (default: this device's Share name, or the room name)"
    )]
    name: Option<String>,
    #[arg(
        long,
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(1..),
        help = "How many minutes the offer stays discoverable"
    )]
    minutes: u64,
    #[arg(
        long,
        value_name = "PIN",
        conflicts_with = "pin_stdin",
        help = "PIN the other device has to enter (visible in the process list)"
    )]
    pin: Option<String>,
    #[arg(long, help = "Read the PIN from stdin (one line)")]
    pin_stdin: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct StopArgs {
    #[arg(
        value_name = "OFFER",
        conflicts_with = "all",
        help = "Offer id or unique id prefix; may be omitted while only one offer runs"
    )]
    offer: Option<String>,
    #[arg(long, help = "Stop every running offer")]
    all: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: DiscoverableArgs) -> Result<(), String> {
    match args.command {
        None => publish(args.publish),
        Some(DiscoverableCommand::List(args)) => list(args.json),
        Some(DiscoverableCommand::Stop(args)) => stop(args),
    }
}

fn publish(args: PublishArgs) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let (target, default_name) = match args.room.as_deref() {
        Some(selector) => {
            let room = resolve_room(&profiles.rooms, selector)?;
            let room_profile_id = room.id.clone();
            let target = DiscoveryPublishTarget::Room { room_profile_id };
            (target, room.name.clone())
        }
        None => {
            let identity = super::identity_command::load_with_repair_hint()?;
            (DiscoveryPublishTarget::Direct, identity.device_name)
        }
    };
    let display_alias = validate_name(args.name.as_deref().unwrap_or(&default_name))?;
    let duration_secs = duration_secs(args.minutes)?;
    let snapshot = crate::daemon::share_worker_snapshot()?;
    if !snapshot.running {
        return Err(INACTIVE_SHARE.to_string());
    }
    let now = crate::share::core_now_secs();
    if let Some(running) = snapshot
        .discovery_offers
        .iter()
        .find(|offer| offer.target == target && offer.discoverable_until > now)
    {
        let subject = match &target {
            DiscoveryPublishTarget::Direct => "this device".to_string(),
            DiscoveryPublishTarget::Room { .. } => target_text(&target, &profiles),
        };
        let until = local_time(running.discoverable_until);
        let offer_id = &running.offer_id;
        return Err(format!(
            "{subject} is already discoverable until {until} (offer {offer_id}); stop it first with `se share discoverable stop {offer_id}`"
        ));
    }
    let stdin_is_terminal = std::io::stdin().is_terminal();
    let pin = read_pin(pin_source(args.pin, args.pin_stdin, stdin_is_terminal)?)?;
    if trivial_pin(&pin) {
        let _ = writeln!(
            std::io::stderr(),
            "se: warning: an empty PIN or \"0\" is trivial to guess"
        );
    }
    let requested = target.clone();
    let command = ShareCmd::Discovery(DiscoveryCommand::Publish {
        target,
        display_alias,
        pin: DiscoveryPin::new(pin),
        duration_secs,
    });
    let offer = match crate::daemon::share_command(command) {
        Ok(ShareCommandReply::DiscoveryOffer { offer }) => offer,
        Ok(ShareCommandReply::DiscoveryOfferEnded { offer_id, reason }) => {
            return Err(format!(
                "offer {offer_id} ended right away: {}",
                stop_reason_text(reason)
            ));
        }
        Ok(other) => return Err(format!("unexpected Share worker reply: {other:?}")),
        Err(error) => return Err(withdraw_unconfirmed(&requested, error)),
    };
    let (offer, connected) = if snapshot.connected {
        await_publication(offer)?
    } else {
        (offer, false)
    };
    print_offer(&offer, &profiles, connected, args.json)
}

/// A publish can fail after the worker already holds the offer (the server
/// connection broke, or the reply timed out); the worker would publish it
/// later and make this device findable with the PIN. Before the publish no
/// offer ran for this target, so any offer for it now is that one.
fn withdraw_unconfirmed(target: &DiscoveryPublishTarget, error: String) -> String {
    let snapshot = match crate::daemon::share_worker_snapshot() {
        Ok(snapshot) => snapshot,
        Err(read_error) => {
            return format!(
                "{error}; whether the worker kept an offer is unknown ({read_error}); check with `se share discoverable list`"
            );
        }
    };
    let mut notes = Vec::new();
    for offer in snapshot.discovery_offers {
        if offer.target != *target {
            continue;
        }
        let offer_id = offer.offer_id;
        let stop = ShareCmd::Discovery(DiscoveryCommand::StopPublishing {
            offer_id: offer_id.clone(),
        });
        notes.push(match crate::daemon::share_command(stop) {
            Ok(_) => format!("the offer {offer_id} the worker kept was stopped"),
            Err(stop_error) => format!(
                "the worker kept offer {offer_id}, and stopping it failed ({stop_error}); stop it with `se share discoverable stop {offer_id}`"
            ),
        });
    }
    if notes.is_empty() {
        error
    } else {
        format!("{error}; {}", notes.join("; "))
    }
}

/// Polls the daemon's persistent offer list until the Share server confirms
/// the offer, the connection drops, or the wait ends.
fn await_publication(offer: OwnDiscoveryOffer) -> Result<(OwnDiscoveryOffer, bool), String> {
    let deadline = Instant::now() + PUBLISH_CONFIRMATION_WAIT;
    let mut current = offer;
    let mut connected = true;
    while !current.published && connected && Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
        let snapshot = crate::daemon::share_worker_snapshot()?;
        connected = snapshot.connected;
        current = snapshot
            .discovery_offers
            .into_iter()
            .find(|offer| offer.offer_id == current.offer_id)
            .ok_or_else(|| {
                format!(
                    "offer {} ended before the Share server confirmed it",
                    current.offer_id
                )
            })?;
    }
    Ok((current, connected))
}

fn print_offer(
    offer: &OwnDiscoveryOffer,
    profiles: &ShareProfiles,
    connected: bool,
    json: bool,
) -> Result<(), String> {
    let now = crate::share::core_now_secs();
    let note = (!offer.published).then(|| pending_note(connected));
    if json {
        let value = serde_json::json!({
            "action": "discoverable",
            "offer": offer_value(offer, profiles, now),
            "note": note,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        println!("{}", offer_text(offer, profiles, now));
        if let Some(note) = note {
            println!("note\t{note}");
        }
    }
    Ok(())
}

fn pending_note(connected: bool) -> &'static str {
    if connected {
        "the Share server has not confirmed the offer yet; the worker keeps publishing it until it ends"
    } else {
        "not connected to the Share server; the worker publishes the offer once it connects"
    }
}

fn list(json: bool) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let now = crate::share::core_now_secs();
    let worker = crate::daemon::share_worker_snapshot();
    let offers: &[OwnDiscoveryOffer] = match &worker {
        Ok(snapshot) => &snapshot.discovery_offers,
        Err(_) => &[],
    };
    if json {
        let value = serde_json::json!({
            "worker": worker_value(&worker),
            "offers": offers
                .iter()
                .map(|offer| offer_value(offer, &profiles, now))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    match &worker {
        Ok(snapshot) => println!(
            "worker\trunning={}\tconnected={}",
            snapshot.running, snapshot.connected
        ),
        Err(error) => println!("worker\tunavailable\t{}", clean(error)),
    }
    for offer in offers {
        println!("{}", offer_text(offer, &profiles, now));
    }
    if worker.is_ok() && offers.is_empty() {
        println!("discoverable\tnone");
    }
    Ok(())
}

fn worker_value(worker: &Result<ShareWorkerSnapshot, String>) -> serde_json::Value {
    match worker {
        Ok(snapshot) => serde_json::json!({
            "reachable": true,
            "running": snapshot.running,
            "connected": snapshot.connected,
            "error": null,
        }),
        Err(error) => serde_json::json!({
            "reachable": false,
            "running": null,
            "connected": null,
            "error": error,
        }),
    }
}

fn stop(args: StopArgs) -> Result<(), String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let selected = select_offers(&snapshot.discovery_offers, args.offer.as_deref(), args.all)?;
    let mut stopped = Vec::new();
    let mut failed = Vec::new();
    for offer in &selected {
        let offer_id = offer.offer_id.clone();
        let command = ShareCmd::Discovery(DiscoveryCommand::StopPublishing {
            offer_id: offer_id.clone(),
        });
        match crate::daemon::share_command(command) {
            Ok(_) => stopped.push(offer_id),
            Err(error) => failed.push((offer_id, error)),
        }
    }
    if args.json {
        let failed_values: Vec<_> = failed
            .iter()
            .map(|(offer_id, error)| serde_json::json!({ "offer_id": offer_id, "error": error }))
            .collect();
        let value = serde_json::json!({
            "action": "stop",
            "stopped": stopped,
            "failed": failed_values,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        if selected.is_empty() {
            println!("stopped\tnone\tno offer is discoverable");
        }
        for offer_id in &stopped {
            println!("stopped\t{offer_id}");
        }
        for (offer_id, error) in &failed {
            println!("failed\t{offer_id}\t{}", clean(error));
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    let (failures, total) = (failed.len(), selected.len());
    Err(format!("{failures} of {total} offers could not be stopped"))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::Cli;

    fn parses(arguments: &str) -> bool {
        Cli::try_parse_from(std::iter::once("se").chain(arguments.split_whitespace())).is_ok()
    }

    #[test]
    fn cli_task_discoverable_parses_one_line_publish_and_subcommands() {
        for accepted in [
            "share discoverable --minutes 5 --pin 1454",
            "share discoverable --pin-stdin --json",
            "share discoverable",
            "share discoverable --room Team --name Laptop",
            "share discoverable list",
            "share discoverable list --json",
            "share discoverable stop",
            "share discoverable stop abc --json",
            "share discoverable stop --all",
        ] {
            assert!(parses(accepted), "failed to parse {accepted:?}");
        }
        for rejected in [
            "share discoverable --minutes 0 --pin 1",
            "share discoverable --pin 1 --pin-stdin",
            "share discoverable --pin 1 list",
            "share discoverable stop abc --all",
        ] {
            assert!(!parses(rejected), "unexpectedly parsed {rejected:?}");
        }
        // An empty PIN is a value the other device can enter, not a missing one.
        assert!(Cli::try_parse_from(["se", "share", "discoverable", "--pin", ""]).is_ok());
    }
}
