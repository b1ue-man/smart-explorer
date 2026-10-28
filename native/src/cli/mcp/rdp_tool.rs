//! Structured inventory and native-client jobs; never a live RDP payload.
//! `open` needs only `device` (design 2.9, user rule r3): relation, fresh
//! authenticated probe, host setup, service, client, certificate trust and
//! the session's life are derived; the session goes to this server's owner
//! registry. `service`, `client` and an explicit `wait` still override;
//! the other arguments of 0.5.188 and earlier stay accepted and are ignored.
use super::{args::Args, response};
use crate::{
    rdp::{AuthenticatedProbe, OpenOptions, SessionView},
    share::{tcp_forward::TunnelId, PeerOpenTarget},
};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

pub(super) const DESCRIPTION: &str = "Remote Desktop to a paired device that granted „Bildschirm steuern“. `open` with only `device` opens the whole desktop in this computer's installed RDP program (optionally `remote_app`: an alias the device publishes); se chooses relation, service and client, sets the device's Remote Desktop up when needed (the device may ask its user once for administrator approval), trusts the certificate from the authenticated probe and keeps the session as long as the RDP program runs. The person types account credentials in the RDP program; this tool takes no password. `local_probe`, `probe` and `clients` read capabilities; `list`, `status` and `close` manage running sessions. Without an RDP program (Android) `open` fails with the reason.";
const IGNORED: &str = "Accepted for callers of 0.5.188 and earlier; ignored.";
pub(super) fn properties() -> Value {
    json!({
        "action":{"type":"string","enum":["local_probe","probe","clients","open","list","status","close"]},
        "device":{"type":"string","description":"probe/open: share endpoint, saved device name, device ID or fingerprint prefix"},
        "remote_app":{"type":"string","description":"open: exact alias of a RemoteApp the device publishes (Windows Server/RDS)"},
        "wait":{"type":"integer","minimum":1,"description":"open: an explicit limit in seconds for the device's answers; without it the call waits until the device answers"},
        "id":{"type":"string","description":"status/close: tunnel ID from open/list"},
        "service":{"type":"string","description":"open: overrides the chosen host service (normally omitted)"},
        "client":{"type":"string","description":"open: overrides the chosen local RDP program (normally omitted)"},
        "certificate_policy":{"type":"string","description":IGNORED},
        "acknowledge_local_access":{"type":"boolean","description":IGNORED},
        "acknowledge_session_effect":{"type":"boolean","description":IGNORED},
        "acknowledge_certificate_dialog":{"type":"boolean","description":IGNORED},
        "lifetime_secs":{"type":"integer","description":IGNORED}
    })
}

/// Resolves a device selector to (device ID, public key).
type Resolve<'a> = dyn FnMut(&str) -> Result<(String, String), String> + 'a;
/// Opens a device and returns its session view: `rdp::open_device_with`
/// in production.
type Open<'a> = dyn FnMut(&str, &str, OpenOptions) -> Result<Value, String> + 'a;

pub(super) fn rdp(
    args: &Args,
    probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Result<CallToolResult, String> {
    let mut resolve = |selector: &str| {
        crate::cli::rdp::device_identity(selector).map(|(device, key, _)| (device, key))
    };
    let mut open = |device: &str, key: &str, options: OpenOptions| {
        let session: SessionView = crate::rdp::open_device_with(device, key, options, || {})?;
        serde_json::to_value(session).map_err(|error| error.to_string())
    };
    rdp_with(args, probe, &mut resolve, &mut open)
}

/// `rdp` with its device resolution and opener injected (tests).
pub(super) fn rdp_with(
    args: &Args,
    mut probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
    resolve: &mut Resolve<'_>,
    open: &mut Open<'_>,
) -> Result<CallToolResult, String> {
    let result = match args.string("action")?.as_str() {
        "local_probe" => {
            serde_json::to_value(crate::rdp::probe_local_host()).map_err(|e| e.to_string())?
        }
        "clients" => json!({"clients":crate::rdp::local_clients()}),
        "probe" => {
            let (target, _) = crate::cli::screen::device_target(&args.string("device")?)?;
            serde_json::to_value(probe(target)?.report()).map_err(|e| e.to_string())?
        }
        "open" => {
            let (device, key) = resolve(&args.string("device")?)?;
            json!({"session": open(&device, &key, open_options(args)?)?})
        }
        "list" => json!({"tunnels":crate::daemon::tcp_list()?}),
        "status" => {
            json!({"tunnel":crate::daemon::tcp_status(TunnelId::parse(&args.string("id")?)?)?})
        }
        "close" => json!({"closed":crate::rdp::close(TunnelId::parse(&args.string("id")?)?)?}),
        action => return Err(format!("Unknown RDP action {action}")),
    };
    Ok(response::ok(result))
}

/// The derived start with the overrides a call may name: `remote_app`,
/// `service`, `client` and an explicit `wait`.
fn open_options(args: &Args) -> Result<OpenOptions, String> {
    let wait = match args.opt_u64("wait")? {
        None => None,
        Some(0) => return Err("wait must be at least one second".into()),
        Some(seconds) => Some(std::time::Duration::from_secs(seconds)),
    };
    Ok(OpenOptions {
        remote_app: args.opt_string("remote_app")?,
        service: args.opt_string("service")?,
        client: args.opt_string("client")?,
        wait,
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "../einrichtung_cli_task_tests.rs"]
mod einrichtung_cli_task_tests;
