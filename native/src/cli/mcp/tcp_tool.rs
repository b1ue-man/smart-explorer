//! MCP uses the daemon token and the remote host's exact TCP grant. This
//! tool cannot answer an approval or mutate RDP, NLA or firewall settings.
use super::{args::Args, response};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelId, TCP_LOCAL_ACCESS_NOTICE};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::time::Duration;

pub(super) const DESCRIPTION: &str = "Start, list, inspect or close a bounded TCP tunnel to one explicitly authorized device service. Start requires acknowledge_local_access=true: every local process can reach its loopback port; the target service must authenticate users itself. Each reverse direction needs a separately authorized tunnel. No SOCKS, UDP, Layer 2, firewall or RDP configuration. Host approval is never answered by this tool.";
pub(super) fn properties() -> Value {
    json!({
        "action": {"type":"string","enum":["start","list","status","close"]},
        "device": {"type":"string","description":"start: share endpoint, saved device name, ID or fingerprint prefix"},
        "target_host": {"type":"string","description":"start: loopback (default), exact IP, or DNS name covered by the host grant"},
        "target_port": {"type":"integer","minimum":1,"maximum":65535},
        "local_port": {"type":"integer","minimum":0,"maximum":65535,"description":"0 (default) selects an ephemeral loopback port"},
        "lifetime_secs": {"type":"integer","minimum":1,"maximum":86400,"default":900},
        "acknowledge_local_access": {"type":"boolean","description":TCP_LOCAL_ACCESS_NOTICE},
        "id": {"type":"string","description":"status/close: stable tunnel ID from start/list"},
        "wait": {"type":"integer","minimum":1,"description":"start: maximum setup/approval wait in seconds, default 30"}
    })
}
pub(super) fn tcp_forward(args: &Args) -> Result<CallToolResult, String> {
    let value = match args.string("action")?.as_str() {
        "list" => json!({"tunnels": crate::daemon::tcp_list()?}),
        "status" => {
            let id = TunnelId::parse(&args.string("id")?)?;
            let tunnel = crate::daemon::tcp_status(id)?
                .ok_or_else(|| "No active TCP tunnel with that ID".to_string())?;
            json!({"tunnel": tunnel})
        }
        "close" => {
            let id = TunnelId::parse(&args.string("id")?)?;
            let closed = crate::daemon::tcp_close(id)?;
            if !closed {
                return Err("No active TCP tunnel with that ID".into());
            }
            json!({"closed": true})
        }
        "start" => {
            let (target, _) = crate::cli::screen::device_target(&args.string("device")?)?;
            let host = args
                .opt_string("target_host")?
                .unwrap_or_else(|| "loopback".into());
            let port = args
                .opt_u64("target_port")?
                .ok_or_else(|| "target_port is required".to_string())?;
            let port =
                u16::try_from(port).map_err(|_| "target_port must be 1..65535".to_string())?;
            let local_port = u16::try_from(args.opt_u64("local_port")?.unwrap_or(0))
                .map_err(|_| "local_port must be 0..65535".to_string())?;
            let lifetime_secs = u32::try_from(args.opt_u64("lifetime_secs")?.unwrap_or(900))
                .map_err(|_| "lifetime_secs must be 1..86400".to_string())?;
            let wait = args.opt_u64("wait")?.unwrap_or(30);
            if wait == 0 {
                return Err("wait must be at least one second".into());
            }
            let request = TcpStart {
                destination: TcpDestination::from_host_port(&host, port)?,
                local_port,
                lifetime_secs,
                acknowledge_local_access: args.flag("acknowledge_local_access")?,
            };
            let tunnel = crate::daemon::tcp_start_within(
                target,
                request,
                Some(Duration::from_secs(wait)),
                || {},
            )?;
            json!({"tunnel": tunnel,"local_access_notice":TCP_LOCAL_ACCESS_NOTICE})
        }
        action => {
            return Err(format!(
                "Unknown TCP action {action}; use start, list, status or close"
            ))
        }
    };
    Ok(response::ok(value))
}
