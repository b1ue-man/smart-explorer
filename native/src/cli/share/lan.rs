//! `se share lan`: local-network presence and automatic uplink sharing.
use clap::{Args, Subcommand};

#[derive(Args)]
#[command(long_about = "Show paired devices found on the local network, the classification of \
every network link, and the state of automatic internet sharing. Without a subcommand this \
prints the status; `presence` and `uplink` change the settings the daemon acts on.")]
pub(super) struct LanArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<LanCommand>,
}

#[derive(Subcommand)]
enum LanCommand {
    #[command(about = "Show LAN presence, links and uplink sharing state")]
    Status,
    #[command(about = "Enable or disable finding paired devices over mDNS")]
    Presence(ToggleArgs),
    #[command(about = "Enable, disable, or stop automatic internet sharing on router-less links")]
    Uplink(UplinkArgs),
}

#[derive(Args)]
struct ToggleArgs {
    #[arg(value_parser = ["on", "off"], help = "on | off")]
    state: String,
}

#[derive(Args)]
struct UplinkArgs {
    #[command(subcommand)]
    command: UplinkCommand,
}

#[derive(Subcommand)]
enum UplinkCommand {
    #[command(about = "Opt in; the daemon runs the one-time platform setup (UAC/polkit) and then shares automatically")]
    Enable,
    #[command(about = "Opt out and stop any active sharing")]
    Disable,
    #[command(about = "Stop the current sharing session; the setting stays enabled")]
    Stop,
    #[command(about = "Show only the uplink sharing state")]
    Status,
}

pub(super) fn run(args: LanArgs) -> Result<(), String> {
    match args.command {
        None | Some(LanCommand::Status) => status(args.json, false),
        Some(LanCommand::Presence(toggle)) => {
            let enabled = toggle.state == "on";
            let settings = crate::share::LanSettings::update(|settings| {
                settings.presence_enabled = enabled;
            })?;
            let (worker_state, worker_error) = worker_refresh();
            print_settings("presence", &settings, worker_state, worker_error, args.json)
        }
        Some(LanCommand::Uplink(uplink)) => match uplink.command {
            UplinkCommand::Enable => {
                let settings = crate::share::LanSettings::update(|settings| {
                    settings.uplink_sharing_enabled = true;
                    settings.uplink_stop_requested_at = None;
                })?;
                let (worker_state, worker_error) = worker_refresh();
                print_settings("uplink_enable", &settings, worker_state, worker_error, args.json)
            }
            UplinkCommand::Disable => {
                let settings = crate::share::LanSettings::update(|settings| {
                    settings.uplink_sharing_enabled = false;
                })?;
                let (worker_state, worker_error) = worker_refresh();
                print_settings("uplink_disable", &settings, worker_state, worker_error, args.json)
            }
            UplinkCommand::Stop => {
                let settings = crate::share::LanSettings::update(|settings| {
                    settings.uplink_stop_requested_at = Some(crate::share::core_now_secs());
                })?;
                let (worker_state, worker_error) = worker_refresh();
                print_settings("uplink_stop", &settings, worker_state, worker_error, args.json)
            }
            UplinkCommand::Status => status(args.json, true),
        },
    }
}

fn status(json: bool, uplink_only: bool) -> Result<(), String> {
    let settings = crate::share::LanSettings::load()?;
    let (lan, worker_error) = match crate::daemon::drain_share_worker_events() {
        Ok(snapshot) => (Some(snapshot.lan), None),
        Err(error) => (None, Some(error)),
    };
    if json {
        let value = serde_json::json!({
            "settings": settings,
            "worker_error": worker_error,
            "lan": lan,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!(
        "settings\tpresence={}\tuplink_sharing={}\tuplink_setup_done={}",
        on_off(settings.presence_enabled),
        on_off(settings.uplink_sharing_enabled),
        on_off(settings.uplink_setup_done)
    );
    let Some(lan) = lan else {
        println!(
            "worker\tunavailable\t{}",
            clean(&worker_error.unwrap_or_default())
        );
        return Ok(());
    };
    if !uplink_only {
        println!(
            "presence\t{}\tannounced_id={}",
            clean(&lan.presence.label()),
            lan.announced_id.as_deref().unwrap_or("-")
        );
        for peer in &lan.peers {
            println!(
                "peer\t{}\tcontact_id={}\tuplink={}\tseen_at={}\tcandidates={}",
                clean(&peer.display_name),
                peer.contact_id,
                peer.uplink.map(on_off).unwrap_or("unknown"),
                peer.seen_at,
                peer.candidates.join(",")
            );
        }
        if let Some(error) = &lan.links_error {
            println!("links_error\t{}", clean(error));
        }
        for link in &lan.links {
            println!(
                "link\t{}\tindex={}\tclass={}\tgateway={}\tdhcp={}\tpeer_present={}\taddrs={}",
                clean(&link.name),
                link.index,
                clean(&link.class),
                on_off(link.has_gateway),
                link.dhcp_lease.map(on_off).unwrap_or("unknown"),
                on_off(link.peer_present),
                link.addrs.join(",")
            );
        }
        if lan.unknown_devices > 0 {
            println!("unknown_devices\t{}", lan.unknown_devices);
        }
    }
    let uplink = &lan.uplink;
    println!(
        "uplink\tenabled={}\tsetup_done={}\tfacility={}\tstate={:?}\treason={}\tpublic_if={}\tprivate_if={}\tsince={}\tlast_error={}",
        on_off(uplink.enabled),
        on_off(uplink.setup_done),
        clean(&uplink.facility.label()),
        uplink.state,
        clean(&uplink.reason),
        uplink.public_if.as_deref().unwrap_or("-"),
        uplink.private_if.as_deref().unwrap_or("-"),
        uplink.since.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
        clean(uplink.last_error.as_deref().unwrap_or("-"))
    );
    Ok(())
}

fn print_settings(
    action: &str,
    settings: &crate::share::LanSettings,
    worker_state: &str,
    worker_error: Option<String>,
    json: bool,
) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": action,
                "settings": settings,
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\t{action}\tpresence={}\tuplink_sharing={}\tworker_refresh={worker_state}",
            on_off(settings.presence_enabled),
            on_off(settings.uplink_sharing_enabled)
        );
        if let Some(error) = worker_error {
            println!("worker_error\t{}", clean(&error));
        }
    }
    Ok(())
}

fn worker_refresh() -> (&'static str, Option<String>) {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => ("refreshed", None),
        Ok(false) => ("inactive", Some("Share worker is not active".to_string())),
        Err(error) => ("unavailable", Some(error)),
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
