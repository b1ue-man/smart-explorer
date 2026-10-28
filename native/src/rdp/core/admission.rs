use super::{
    CertificatePolicy, ClientCapability, ClientKind, HostProbe, HostService, PublishedApp,
};
use crate::share::tcp_forward::{TcpDestination, TcpStart, TunnelRole, TunnelView};
use serde::{Deserialize, Serialize};

/// Not deserializable. Only the authenticated peer transport constructs this
/// wrapper after binding the report to the peer node used by the TCP tunnel.
/// One probe serves one start: `RdpSession::start` takes it by value
/// (design 2.1, replaces the former 300-second age limit).
pub(crate) struct AuthenticatedProbe {
    node_id: String,
    report: HostProbe,
}
impl AuthenticatedProbe {
    /// Caller must have verified the existing peer principal and relationship.
    /// JSON input from CLI/MCP/Bridge is never an authenticated peer response.
    pub(crate) fn from_authenticated_peer(
        node_id: String,
        report: HostProbe,
    ) -> Result<Self, String> {
        if node_id.is_empty() || node_id.len() > 256 || node_id.chars().any(char::is_control) {
            return Err("RDP report has no authenticated peer node identity".into());
        }
        report.validate()?;
        Ok(Self { node_id, report })
    }
    pub(crate) fn report(&self) -> &HostProbe {
        &self.report
    }
    pub(crate) fn node_id(&self) -> &str {
        &self.node_id
    }
    pub(crate) fn service(&self, id: &str) -> Result<&HostService, String> {
        self.report
            .services
            .iter()
            .find(|service| service.id == id)
            .ok_or_else(|| "RDP service is absent from the authenticated inventory".into())
    }
    /// Design 2.3: the requested service, otherwise the startable service of
    /// the smallest rank.
    pub(crate) fn choose_service(&self, wanted: Option<&str>) -> Result<&HostService, String> {
        match wanted {
            Some(id) => self.service(id),
            None => self.report.preferred_service().ok_or_else(|| {
                let reasons: Vec<String> = self
                    .report
                    .services
                    .iter()
                    .filter_map(|service| service.startable().err())
                    .chain(self.report.unavailable_reason.clone())
                    .collect();
                if reasons.is_empty() {
                    "Das Gerät meldet keinen startbaren Remote-Desktop-Dienst".to_string()
                } else {
                    format!(
                        "Kein startbarer Remote-Desktop-Dienst: {}",
                        reasons.join("; ")
                    )
                }
            }),
        }
    }
    pub(crate) fn check_tunnel(
        &self,
        service: &HostService,
        tunnel: &TunnelView,
    ) -> Result<(), String> {
        let port = service
            .port
            .ok_or_else(|| "RDP listener port is unknown".to_string())?;
        let expected = TcpDestination::from_host_port("loopback", port)?;
        if tunnel.role != TunnelRole::Listener
            || tunnel.remote_node != self.node_id
            || tunnel.target_node != self.node_id
            || tunnel.destination != expected
            || tunnel.closing
            || tunnel
                .listener
                .is_none_or(|a| !a.ip().is_loopback() || a.port() == 0)
        {
            return Err(
                "RDP tunnel does not match the authenticated peer and exact loopback service"
                    .into(),
            );
        }
        Ok(())
    }
}

/// What starts one native client. Every field is derived (design 2.1–2.7);
/// fields of older callers (`acknowledge_*`, `lifetime_secs`) are accepted
/// in JSON and ignored: the one grant „Bildschirm steuern: Erlauben“ is the
/// host's consent, the listener admits only the started client, the tunnel
/// lives as long as that client and its connection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StartRequest {
    pub(crate) service_id: String,
    pub(crate) client_id: String,
    /// The policy of the chosen client (`ClientCapability::certificate_policy`).
    pub(crate) certificate_policy: CertificatePolicy,
    pub(crate) remote_app: Option<String>,
}
impl StartRequest {
    /// The request for `service` started with `client`.
    pub(crate) fn derive(
        service: &HostService,
        client: &ClientCapability,
        remote_app: Option<String>,
    ) -> Self {
        Self {
            service_id: service.id.clone(),
            client_id: client.id.clone(),
            certificate_policy: client.certificate_policy,
            remote_app,
        }
    }
    pub(crate) fn validate<'a>(
        &self,
        probe: &'a AuthenticatedProbe,
    ) -> Result<&'a HostService, String> {
        let service = probe.service(&self.service_id)?;
        service.startable()?;
        if self.certificate_policy == CertificatePolicy::ExactPeerPin {
            return Err("No installed-client adapter proves exclusive peer-certificate pinning; select an explicitly documented verification policy".into());
        }
        if service.certificate_sha256.is_none() {
            return Err(
                "The host has not supplied a proved SHA-256 RDP certificate fingerprint".into(),
            );
        }
        if let Some(alias) = &self.remote_app {
            if !PublishedApp::valid_alias(alias)
                || !service.remote_apps.iter().any(|a| a.alias == *alias)
            {
                return Err(
                    "RemoteApp alias was not published by this Windows Server RDS host".into(),
                );
            }
        }
        self.tunnel_request(service)?.validate()?;
        Ok(service)
    }
    /// Evidence a client needs beyond the common checks: mstsc trusts the
    /// host through the stored SHA-1 `CertHash` (design 2.7.2), which only a
    /// probe v2 host delivers. No return to the native certificate dialog.
    pub(crate) fn require_client_evidence(
        service: &HostService,
        client: &ClientCapability,
    ) -> Result<(), String> {
        if client.kind == ClientKind::Mstsc && service.certificate_sha1.is_none() {
            return Err(crate::rdp::OLD_HOST.into());
        }
        Ok(())
    }
    /// The tunnel to the host's RDP port: no fixed lifetime (design 2.6).
    pub(crate) fn tunnel_request(&self, service: &HostService) -> Result<TcpStart, String> {
        Ok(TcpStart {
            destination: TcpDestination::from_host_port(
                "loopback",
                service
                    .port
                    .ok_or_else(|| "RDP host listener port is unknown".to_string())?,
            )?,
            local_port: 0,
            acknowledge_local_access: true,
            lifetime_secs: 0,
        })
    }
}
