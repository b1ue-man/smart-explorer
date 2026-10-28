//! `se tcp start/services/list/status/close`, with identical JSON and daemon
//! semantics (design 4, user rule r4 „tcp, alles“). `services` lists the
//! locally listening services a device shares under „Bildschirm steuern:
//! Erlauben“ and their address on this device. `start` needs no
//! acknowledgement (the listener admits only this account's programs) and
//! has no fixed lifetime unless `--lifetime-secs` names one (user rule r7).
use crate::desktop::core::tiles;
use crate::share::apps_wire::{AppsCommand, AppsResult};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelId, TunnelView};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

#[derive(Args)]
pub(super) struct TcpArgs {
    #[command(subcommand)]
    command: TcpCommand,
}
#[derive(Subcommand)]
enum TcpCommand {
    /// Expose one authorized service through a local loopback port.
    Start(StartArgs),
    /// The services a device shares with this one and their address here.
    Services {
        device: String,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Status {
        id: String,
        #[arg(long)]
        json: bool,
    },
    Close {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Debug, Default, PartialEq, Eq)]
pub(crate) struct StartArgs {
    pub(crate) device: String,
    #[arg(long, default_value = "loopback")]
    pub(crate) target_host: String,
    #[arg(long)]
    pub(crate) target_port: u16,
    #[arg(long, default_value_t = 0)]
    pub(crate) local_port: u16,
    /// 0 (default): no fixed lifetime; otherwise 1..86400 seconds.
    #[arg(long, default_value_t = 0)]
    pub(crate) lifetime_secs: u32,
    /// Accepted for scripts of 0.5.188 and earlier; no longer needed.
    #[arg(long, hide = true)]
    pub(crate) acknowledge_local_access: bool,
    #[arg(long)]
    pub(crate) json: bool,
}

impl StartArgs {
    /// The start request: the acknowledgement field is always set, because
    /// the listener itself checks every local connection's account.
    pub(crate) fn request(&self) -> Result<TcpStart, String> {
        let request = TcpStart {
            destination: TcpDestination::from_host_port(&self.target_host, self.target_port)?,
            local_port: self.local_port,
            acknowledge_local_access: true,
            lifetime_secs: self.lifetime_secs,
        };
        request.validate()?;
        Ok(request)
    }
}

pub(super) fn run(args: TcpArgs) -> Result<i32, String> {
    match args.command {
        TcpCommand::Start(start) => {
            let (target, name) = super::screen::device_target(&start.device)?;
            let request = start.request()?;
            let tunnel = crate::daemon::tcp_start(target, request, || {
                eprintln!("Warte auf die Antwort von {name} …");
            })?;
            output(start.json, &json!({"tunnel": tunnel}), || text(&tunnel));
        }
        TcpCommand::Services { device, json } => {
            let value = services(&device)?;
            output(json, &value, || services_text(&value));
        }
        TcpCommand::List { json } => {
            let tunnels = crate::daemon::tcp_list()?;
            output(json, &json!({"tunnels": tunnels}), || {
                tunnels.iter().map(text).collect::<Vec<_>>().join("\n")
            });
        }
        TcpCommand::Status { id, json } => {
            let tunnel = crate::daemon::tcp_status(TunnelId::parse(&id)?)?
                .ok_or_else(|| format!("No active TCP tunnel {id}"))?;
            output(json, &json!({"tunnel": tunnel}), || text(&tunnel));
        }
        TcpCommand::Close { id, json } => {
            let closed = crate::daemon::tcp_close(TunnelId::parse(&id)?)?;
            if !closed {
                return Err(format!("No active TCP tunnel {id}"));
            }
            output(json, &json!({"id": id, "closed": true}), || {
                format!("closed\t{id}")
            });
        }
    }
    Ok(0)
}

/// `{"device","services":[{port,scope,owner,process,pid,address,url}],
/// "unavailable"}`: the device's own answer (`LocalServices`, design 9.2)
/// with the address the background service bound for each port here.
pub(crate) fn services(selector: &str) -> Result<Value, String> {
    let (target, name) = super::screen::device_target(selector)?;
    let snapshot = crate::daemon::share_worker_snapshot()?;
    let feed = tiles::identity_of(&snapshot.profiles, &target)
        .and_then(|(device, key)| tiles::feed_of(&snapshot, &device, &key));
    let waiting = || eprintln!("Warte auf die Antwort von {name} …");
    let (_, _, result) =
        crate::daemon::apps_command(target, AppsCommand::LocalServices, None, waiting)?;
    let AppsResult::LocalServices {
        services,
        unavailable,
    } = result
    else {
        return Err("Unerwartete Antwort auf die Dienstabfrage.".into());
    };
    Ok(services_value(&name, &services, unavailable, feed.as_ref()))
}

/// The JSON of `se tcp services` from the device's answer and this device's
/// feed projection (testable without a device).
pub(crate) fn services_value(
    name: &str,
    services: &[crate::share::loopback_services::LoopbackService],
    unavailable: Option<String>,
    feed: Option<&tiles::TileFeed>,
) -> Value {
    let rows: Vec<Value> = services
        .iter()
        .map(|service| {
            let mapped = feed
                .and_then(|feed| feed.services.iter().find(|s| s.port == service.port))
                .cloned();
            let mut row = serde_json::to_value(service).unwrap_or(Value::Null);
            if let Value::Object(map) = &mut row {
                let address = mapped.as_ref().and_then(|m| m.address);
                map.insert("address".into(), json!(address));
                map.insert("url".into(), json!(mapped.and_then(|m| m.url)));
            }
            row
        })
        .collect();
    json!({"device": name, "services": rows, "unavailable": unavailable})
}

fn services_text(value: &Value) -> String {
    let field = |row: &Value, key: &str| match &row[key] {
        Value::Null => "-".to_string(),
        Value::String(text) => text.replace(['\t', '\r', '\n'], " "),
        other => other.to_string(),
    };
    let mut lines: Vec<String> = value["services"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    format!(
                        "service\t{}\t{}\t{}\t{}",
                        field(row, "port"),
                        field(row, "process"),
                        field(row, "url"),
                        field(row, "address")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(reason) = value["unavailable"].as_str() {
        lines.push(format!("unavailable\t{reason}"));
    }
    lines.join("\n")
}

fn text(tunnel: &TunnelView) -> String {
    format!("id={} role={:?} peer={} initiator={} target={} direction={:?} relation={}:{} destination={} listener={} bound_target={} started_at={} expires_at={} admission={} active_connections={} socket_read_bytes={} socket_written_bytes={} closing={} last_error={}",
        tunnel.id.as_str(), tunnel.role, tunnel.remote_node,
        tunnel.initiator_node, tunnel.target_node, tunnel.direction, tunnel.relation_kind, tunnel.relation_id,
        serde_json::to_string(&tunnel.destination).unwrap_or_default(),
        tunnel.listener.map_or_else(|| "-".into(), |address| address.to_string()),
        tunnel.bound_target.map_or_else(|| "-".into(), |address| address.to_string()),
        tunnel.started_at,
        tunnel.expires_at.map_or_else(|| "-".into(), |at| at.to_string()),
        tunnel.admission.map_or_else(|| "-".into(), |a| format!("{a:?}")),
        tunnel.active_connections,
        tunnel.socket_read_bytes, tunnel.socket_written_bytes, tunnel.closing,
        tunnel.last_error.as_deref().unwrap_or(""))
}
fn output(json: bool, value: &Value, text: impl FnOnce() -> String) {
    if json {
        println!("{value}");
    } else {
        println!("{}", text());
    }
}
