//! Structured inventory and native-client jobs; never a live RDP payload.
use super::{args::Args, response};
use crate::{rdp::{AuthenticatedProbe, CertificatePolicy, StartRequest}, share::{PeerOpenTarget, tcp_forward::TunnelId}};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

pub(super) const DESCRIPTION: &str = "Read actual RDP host/client capability, open a native installed RDP client or close its bounded TCP lease. Host certificate and published RemoteApp aliases are obtained only through the authenticated peer probe. Start requires acknowledge_local_access and acknowledge_session_effect. Native certificate dialogs require the user's own fingerprint comparison. No password arguments, client installation, certificate bypass or RDP/firewall/NLA mutation. Exclusive peer pinning is unavailable when the installed client cannot enforce it.";
pub(super) fn properties() -> Value {
    json!({
        "action":{"type":"string","enum":["local_probe","probe","clients","open","list","status","close"]},
        "device":{"type":"string"},"service":{"type":"string"},"client":{"type":"string"},
        "certificate_policy":{"type":"string","enum":["exact_peer_pin","peer_pin_or_native_trust","native_dialog"]},
        "remote_app":{"type":"string","description":"Exact alias from the authenticated Windows Server RDS publication inventory"},
        "acknowledge_local_access":{"type":"boolean","description":crate::share::tcp_forward::TCP_LOCAL_ACCESS_NOTICE},
        "acknowledge_session_effect":{"type":"boolean"},"acknowledge_certificate_dialog":{"type":"boolean"},
        "lifetime_secs":{"type":"integer","minimum":1,"maximum":86400,"default":900},
        "wait":{"type":"integer","minimum":1,"default":30},"id":{"type":"string"}
    })
}
pub(super) fn rdp(args: &Args, mut probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>) -> Result<CallToolResult, String> {
    let result = match args.string("action")?.as_str() {
        "local_probe" => serde_json::to_value(crate::rdp::probe_local_host()).map_err(|e| e.to_string())?,
        "clients" => json!({"clients":crate::rdp::local_clients()}),
        "probe" => {
            let (target, _) = crate::cli::screen::device_target(&args.string("device")?)?;
            serde_json::to_value(probe(target)?.report()).map_err(|e| e.to_string())?
        }
        "open" => {
            let (target, _) = crate::cli::screen::device_target(&args.string("device")?)?;
            let report = probe(target.clone())?;
            let policy = match args.opt_string("certificate_policy")?.as_deref().unwrap_or("exact_peer_pin") {
                "exact_peer_pin" => CertificatePolicy::ExactPeerPin,
                "peer_pin_or_native_trust" => CertificatePolicy::PeerPinOrNativeTrust,
                "native_dialog" => CertificatePolicy::NativeDialog,
                _ => return Err("Invalid RDP certificate_policy".into()),
            };
            let request = StartRequest { service_id:args.string("service")?,client_id:args.string("client")?,
                certificate_policy:policy,remote_app:args.opt_string("remote_app")?,
                acknowledge_local_access:args.flag("acknowledge_local_access")?,
                acknowledge_session_effect:args.flag("acknowledge_session_effect")?,
                acknowledge_certificate_dialog:args.flag("acknowledge_certificate_dialog")?,
                lifetime_secs:u32::try_from(args.opt_u64("lifetime_secs")?.unwrap_or(900))
                    .map_err(|_| "lifetime_secs must be 1..86400".to_string())? };
            let wait = args.opt_u64("wait")?.unwrap_or(30);
            if wait == 0 || wait > 300 { return Err("RDP setup wait must be 1..300 seconds".into()); }
            let session = crate::rdp::start(target,&report,&request,std::time::Duration::from_secs(wait),|| {})?;
            json!({"session":crate::rdp::own_in_background(session)?})
        }
        "list" => json!({"tunnels":crate::daemon::tcp_list()?}),
        "status" => json!({"tunnel":crate::daemon::tcp_status(TunnelId::parse(&args.string("id")?)?)?}),
        "close" => json!({"closed":crate::rdp::close(TunnelId::parse(&args.string("id")?)?)?}),
        action => return Err(format!("Unknown RDP action {action}")),
    };
    Ok(response::ok(result))
}
