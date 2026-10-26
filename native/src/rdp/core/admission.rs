use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use super::{CertificatePolicy, HostProbe, HostService, PublishedApp};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelRole, TunnelView};

/// Not deserializable. Only the authenticated peer transport constructs this
/// wrapper after binding the report to the peer node used by the TCP tunnel.
pub(crate) struct AuthenticatedProbe {
    node_id: String,
    report: HostProbe,
    received: Instant,
}
impl AuthenticatedProbe {
    /// Caller must have verified the existing peer principal and relationship.
    /// JSON input from CLI/MCP/Bridge is never an authenticated peer response.
    pub(crate) fn from_authenticated_peer(node_id: String, report: HostProbe) -> Result<Self, String> {
        if node_id.is_empty() || node_id.len() > 256 || node_id.chars().any(char::is_control) {
            return Err("RDP report has no authenticated peer node identity".into());
        }
        report.validate()?;
        Ok(Self { node_id, report, received: Instant::now() })
    }
    pub(crate) fn report(&self) -> &HostProbe { &self.report }
    pub(crate) fn service(&self, id: &str) -> Result<&HostService, String> {
        // The adapter permits one setup/approval window, not a persisted pin.
        if self.received.elapsed() > Duration::from_secs(300) {
            return Err("RDP host inventory expired; probe the authenticated peer again".into());
        }
        self.report.services.iter().find(|service| service.id == id)
            .ok_or_else(|| "RDP service is absent from the authenticated inventory".into())
    }
    pub(crate) fn check_tunnel(&self, service: &HostService, tunnel: &TunnelView) -> Result<(), String> {
        let port = service.port.ok_or_else(|| "RDP listener port is unknown".to_string())?;
        let expected = TcpDestination::from_host_port("loopback", port)?;
        if tunnel.role != TunnelRole::Listener || tunnel.remote_node != self.node_id
            || tunnel.target_node != self.node_id || tunnel.destination != expected
            || tunnel.closing || tunnel.listener.is_none_or(|a| !a.ip().is_loopback() || a.port() == 0) {
            return Err("RDP tunnel does not match the authenticated peer and exact loopback service".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartRequest {
    pub(crate) service_id: String,
    pub(crate) client_id: String,
    pub(crate) certificate_policy: CertificatePolicy,
    pub(crate) remote_app: Option<String>,
    #[serde(default)]
    pub(crate) acknowledge_local_access: bool,
    #[serde(default)]
    pub(crate) acknowledge_session_effect: bool,
    #[serde(default)]
    pub(crate) acknowledge_certificate_dialog: bool,
    #[serde(default = "default_lifetime")]
    pub(crate) lifetime_secs: u32,
}
fn default_lifetime() -> u32 { 900 }
impl StartRequest {
    pub(crate) fn validate<'a>(&self, probe: &'a AuthenticatedProbe) -> Result<&'a HostService, String> {
        let service = probe.service(&self.service_id)?;
        service.startable()?;
        if !self.acknowledge_session_effect { return Err(service.session_mode.notice().into()); }
        if self.certificate_policy == CertificatePolicy::ExactPeerPin {
            return Err("No installed-client adapter proves exclusive peer-certificate pinning; select an explicitly documented verification policy".into());
        }
        if service.certificate_sha256.is_none() {
            return Err("The host has not supplied a proved SHA-256 RDP certificate fingerprint".into());
        }
        if self.certificate_policy == CertificatePolicy::NativeDialog && !self.acknowledge_certificate_dialog {
            return Err("Compare the host SHA-256 fingerprint in the native certificate dialog before accepting; explicit acknowledgement is required".into());
        }
        if let Some(alias) = &self.remote_app {
            if !PublishedApp::valid_alias(alias) || !service.remote_apps.iter().any(|a| a.alias == *alias) {
                return Err("RemoteApp alias was not published by this Windows Server RDS host".into());
            }
        }
        self.tunnel_request(service)?.validate()?;
        Ok(service)
    }
    pub(crate) fn tunnel_request(&self, service: &HostService) -> Result<TcpStart, String> {
        Ok(TcpStart { destination: TcpDestination::from_host_port("loopback", service.port
            .ok_or_else(|| "RDP host listener port is unknown".to_string())?)?,
            local_port: 0, acknowledge_local_access: self.acknowledge_local_access,
            lifetime_secs: self.lifetime_secs })
    }
}
