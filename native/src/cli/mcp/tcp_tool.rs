//! MCP uses the daemon token and the remote host's grant. `services` lists
//! what a device shares under „Bildschirm steuern: Erlauben“ and where it is
//! reachable here (design 4.3); `start` needs no acknowledgement and has no
//! fixed lifetime unless `lifetime_secs` names one (design 4.4, user rule
//! r7). This tool cannot answer an approval or mutate RDP, NLA or firewall
//! settings.
use super::{args::Args, response};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelId};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::time::Duration;

pub(super) const DESCRIPTION: &str = "TCP to paired devices. `services` lists the locally listening services a device shares with this one because it granted „Bildschirm steuern“, each with its address on this computer (the device's own loopback address, same port); programs of this user reach them there directly, nothing to start. `start` opens a further tunnel to one service the device's TCP grant covers (its network, a DNS name or a port it does not share): only programs of this user account reach the local port, and it stays open until closed, unless `lifetime_secs` names a limit. `list`, `status` and `close` manage tunnels. No SOCKS, UDP, Layer 2, firewall or RDP configuration. Host approval is never answered by this tool.";
pub(super) fn properties() -> Value {
    json!({
        "action": {"type":"string","enum":["services","start","list","status","close"]},
        "device": {"type":"string","description":"services/start: share endpoint, saved device name, ID or fingerprint prefix"},
        "target_host": {"type":"string","description":"start: loopback (default), exact IP, or DNS name covered by the host grant"},
        "target_port": {"type":"integer","minimum":1,"maximum":65535},
        "local_port": {"type":"integer","minimum":0,"maximum":65535,"description":"0 (default) selects an ephemeral loopback port"},
        "lifetime_secs": {"type":"integer","minimum":0,"maximum":86400,"description":"start: 0 or absent = no fixed lifetime; otherwise the tunnel ends after this many seconds"},
        "acknowledge_local_access": {"type":"boolean","description":"Accepted for callers of 0.5.188 and earlier; ignored."},
        "id": {"type":"string","description":"status/close: stable tunnel ID from start/list"},
        "wait": {"type":"integer","minimum":1,"description":"start: maximum wait in seconds for the device's approval; without it the call waits until the device answers"}
    })
}
pub(super) fn tcp_forward(args: &Args) -> Result<CallToolResult, String> {
    let value = match args.string("action")?.as_str() {
        "services" => crate::cli::tcp::services(&args.string("device")?)?,
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
            let request = start_request(args)?;
            let tunnel = crate::daemon::tcp_start_within(target, request, wait(args)?, || {})?;
            json!({"tunnel": tunnel})
        }
        action => {
            return Err(format!(
                "Unknown TCP action {action}; use services, start, list, status or close"
            ))
        }
    };
    Ok(response::ok(value))
}

/// The start request of a call: no acknowledgement needed, no lifetime
/// unless one is named.
pub(super) fn start_request(args: &Args) -> Result<TcpStart, String> {
    let host = args
        .opt_string("target_host")?
        .unwrap_or_else(|| "loopback".into());
    let port = args
        .opt_u64("target_port")?
        .ok_or_else(|| "target_port is required".to_string())?;
    let port = u16::try_from(port).map_err(|_| "target_port must be 1..65535".to_string())?;
    let local_port = u16::try_from(args.opt_u64("local_port")?.unwrap_or(0))
        .map_err(|_| "local_port must be 0..65535".to_string())?;
    let lifetime_secs = u32::try_from(args.opt_u64("lifetime_secs")?.unwrap_or(0))
        .map_err(|_| "lifetime_secs must be 0..86400".to_string())?;
    let request = TcpStart {
        destination: TcpDestination::from_host_port(&host, port)?,
        local_port,
        lifetime_secs,
        acknowledge_local_access: true,
    };
    request.validate()?;
    Ok(request)
}

/// An explicitly named wait; `None` waits until the device answers.
fn wait(args: &Args) -> Result<Option<Duration>, String> {
    match args.opt_u64("wait")? {
        None => Ok(None),
        Some(0) => Err("wait must be at least one second".into()),
        Some(seconds) => Ok(Some(Duration::from_secs(seconds))),
    }
}
