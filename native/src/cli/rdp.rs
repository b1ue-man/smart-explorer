//! `se rdp`: explicit native RDP path; the authenticated probe is injected
//! by the daemon integration. No CLI-supplied inventory or certificate pin.
use crate::{
    rdp::{AuthenticatedProbe, CertificatePolicy, StartRequest},
    share::PeerOpenTarget,
};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Args)]
pub(super) struct RdpArgs {
    #[command(subcommand)]
    command: RdpCommand,
}
#[derive(Subcommand)]
enum RdpCommand {
    /// Read this host's real RDP services without changing its setup.
    LocalProbe {
        #[arg(long)]
        json: bool,
    },
    Probe {
        device: String,
        #[arg(long)]
        json: bool,
    },
    Clients {
        #[arg(long)]
        json: bool,
    },
    /// Open the selected installed client; this command owns it until exit.
    Open {
        device: String,
        #[arg(long)]
        service: String,
        #[arg(long)]
        client: String,
        /// exact-peer-pin, peer-pin-or-native-trust, or native-dialog.
        #[arg(long, default_value = "exact-peer-pin")]
        certificate_policy: String,
        #[arg(long)]
        remote_app: Option<String>,
        #[arg(long)]
        acknowledge_local_access: bool,
        #[arg(long)]
        acknowledge_session_effect: bool,
        #[arg(long)]
        acknowledge_certificate_dialog: bool,
        #[arg(long, default_value_t = 900)]
        lifetime_secs: u32,
        #[arg(long, default_value_t = 30)]
        wait: u32,
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
pub(crate) fn certificate_policy(value: &str) -> Result<CertificatePolicy, String> {
    match value {
        "exact-peer-pin" | "exact_peer_pin" => Ok(CertificatePolicy::ExactPeerPin),
        "peer-pin-or-native-trust" | "peer_pin_or_native_trust" => {
            Ok(CertificatePolicy::PeerPinOrNativeTrust)
        }
        "native-dialog" | "native_dialog" => Ok(CertificatePolicy::NativeDialog),
        _ => Err(
            "certificate_policy must be exact-peer-pin, peer-pin-or-native-trust or native-dialog"
                .into(),
        ),
    }
}
pub(super) fn run(
    args: RdpArgs,
    mut probe: impl FnMut(PeerOpenTarget) -> Result<AuthenticatedProbe, String>,
) -> Result<i32, String> {
    use crate::share::tcp_forward::TunnelId;
    match args.command {
        RdpCommand::LocalProbe { json } => {
            let value = crate::rdp::probe_local_host();
            output(
                json,
                &serde_json::to_value(&value).map_err(|e| e.to_string())?,
            );
        }
        RdpCommand::Probe { device, json } => {
            let (target, _) = super::screen::device_target(&device)?;
            let report = probe(target)?;
            output(
                json,
                &serde_json::to_value(report.report()).map_err(|e| e.to_string())?,
            );
        }
        RdpCommand::Clients { json } => {
            output(json, &json!({"clients":crate::rdp::local_clients()}))
        }
        RdpCommand::Open {
            device,
            service,
            client,
            certificate_policy: policy,
            remote_app,
            acknowledge_local_access,
            acknowledge_session_effect,
            acknowledge_certificate_dialog,
            lifetime_secs,
            wait,
        } => {
            if wait == 0 {
                return Err("RDP setup wait must be at least one second".into());
            }
            let (target, name) = super::screen::device_target(&device)?;
            let report = probe(target.clone())?;
            let request = StartRequest {
                service_id: service,
                client_id: client,
                certificate_policy: certificate_policy(&policy)?,
                remote_app,
                acknowledge_local_access,
                acknowledge_session_effect,
                acknowledge_certificate_dialog,
                lifetime_secs,
            };
            let service = request.validate(&report)?;
            eprintln!(
                "{}\nSHA-256: {}\n{}",
                service.session_mode.notice(),
                service
                    .certificate_sha256
                    .as_ref()
                    .map_or("unavailable", |pin| pin.as_str()),
                crate::share::tcp_forward::TCP_LOCAL_ACCESS_NOTICE
            );
            let mut session = crate::rdp::start(
                target,
                &report,
                &request,
                std::time::Duration::from_secs(wait.into()),
                || {
                    eprintln!("Warte auf TCP-Bestätigung an {name} …");
                },
            )?;
            eprintln!(
                "RDP tunnel={} client_pid={}",
                session.tunnel_id().as_str(),
                session.view().process_id
            );
            let status = session.wait()?;
            if !status.success() {
                return Err(format!("Native RDP client exited with {status}"));
            }
        }
        RdpCommand::List { json } => output(json, &json!({"tunnels":crate::daemon::tcp_list()?})),
        RdpCommand::Status { id, json } => output(
            json,
            &json!({"tunnel":crate::daemon::tcp_status(TunnelId::parse(&id)?)?}),
        ),
        RdpCommand::Close { id, json } => output(
            json,
            &json!({"id":id,"closed":crate::rdp::close(TunnelId::parse(&id)?)?}),
        ),
    }
    Ok(0)
}
fn output(json: bool, value: &serde_json::Value) {
    if json {
        println!("{value}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    }
}
