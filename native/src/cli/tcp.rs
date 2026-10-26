//! `se tcp start/list/status/close`, with identical JSON and daemon semantics.
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelId, TunnelView, TCP_LOCAL_ACCESS_NOTICE};

#[derive(Args)]
pub(super) struct TcpArgs { #[command(subcommand)] command: TcpCommand }
#[derive(Subcommand)]
enum TcpCommand {
    /// Expose one authorized service through an ephemeral local loopback port.
    Start {
        device: String,
        #[arg(long, default_value = "loopback")] target_host: String,
        #[arg(long)] target_port: u16,
        #[arg(long, default_value_t = 0)] local_port: u16,
        #[arg(long, default_value_t = 900)] lifetime_secs: u32,
        /// Confirm that every local process can reach the forwarded service.
        #[arg(long)] acknowledge_local_access: bool,
        #[arg(long)] json: bool,
    },
    List { #[arg(long)] json: bool },
    Status { id: String, #[arg(long)] json: bool },
    Close { id: String, #[arg(long)] json: bool },
}
pub(super) fn run(args: TcpArgs) -> Result<i32, String> {
    match args.command {
        TcpCommand::Start { device, target_host, target_port, local_port, lifetime_secs, acknowledge_local_access, json } => {
            let (target, name) = super::screen::device_target(&device)?;
            let request = TcpStart {
                destination: TcpDestination::from_host_port(&target_host, target_port)?, local_port,
                acknowledge_local_access, lifetime_secs,
            };
            request.validate()?;
            let tunnel = crate::daemon::tcp_start(target, request, || {
                eprintln!("Warte auf TCP-Bestätigung an {name} …");
            })?;
            output(json, &json!({"tunnel": tunnel, "local_access_notice": TCP_LOCAL_ACCESS_NOTICE}), || text(&tunnel));
        }
        TcpCommand::List { json } => {
            let tunnels = crate::daemon::tcp_list()?;
            output(json, &json!({"tunnels": tunnels}), || tunnels.iter().map(text).collect::<Vec<_>>().join("\n"));
        }
        TcpCommand::Status { id, json } => {
            let tunnel = crate::daemon::tcp_status(TunnelId::parse(&id)?)?
                .ok_or_else(|| format!("No active TCP tunnel {id}"))?;
            output(json, &json!({"tunnel": tunnel}), || text(&tunnel));
        }
        TcpCommand::Close { id, json } => {
            let closed = crate::daemon::tcp_close(TunnelId::parse(&id)?)?;
            if !closed { return Err(format!("No active TCP tunnel {id}")); }
            output(json, &json!({"id": id, "closed": true}), || format!("closed\t{id}"));
        }
    }
    Ok(0)
}
fn text(tunnel: &TunnelView) -> String {
    format!("id={} role={:?} peer={} initiator={} target={} direction={:?} relation={}:{} destination={} listener={} bound_target={} started_at={} expires_at={} active_connections={} socket_read_bytes={} socket_written_bytes={} closing={} last_error={}",
        tunnel.id.as_str(), tunnel.role, tunnel.remote_node,
        tunnel.initiator_node, tunnel.target_node, tunnel.direction, tunnel.relation_kind, tunnel.relation_id,
        serde_json::to_string(&tunnel.destination).unwrap_or_default(),
        tunnel.listener.map_or_else(|| "-".into(), |address| address.to_string()),
        tunnel.bound_target.map_or_else(|| "-".into(), |address| address.to_string()),
        tunnel.started_at, tunnel.expires_at, tunnel.active_connections,
        tunnel.socket_read_bytes, tunnel.socket_written_bytes, tunnel.closing,
        tunnel.last_error.as_deref().unwrap_or(""))
}
fn output(json: bool, value: &Value, text: impl FnOnce() -> String) {
    if json { println!("{value}"); } else { println!("{}", text()); }
}
