//! Host side of design 9.2: RDP probe v1/v2, the RDP self-setup on a
//! viewer's request (T4), se-managed RDP credentials and the list of local
//! services that „Bildschirm steuern: Erlauben“ exposes. Probes are admitted
//! like the v1 probe (screen-view metadata); everything else needs steering.
use super::super::*;
use crate::rdp::core::SetupTrigger;
use crate::share::tcp_sockets::local_services;

impl ShareIrohNode {
    pub(in super::super) async fn apps_einrichtung(
        self: &Arc<Self>,
        principal: &ProvenPrincipal,
        command: &AppsCommand,
        send: &mut iroh::endpoint::SendStream,
    ) -> io::Result<AppsResult> {
        match command {
            AppsCommand::RdpProbe => {
                self.require_inventory(principal)?;
                let host = probe().await?;
                self.require_inventory(principal)?;
                Ok(AppsResult::RdpProbe {
                    host: host.into_v1(),
                })
            }
            AppsCommand::RdpProbeV2 => {
                self.require_inventory(principal)?;
                let host = probe_v2().await?;
                let setup = crate::rdp::host_setup::current_state(&host);
                self.require_inventory(principal)?;
                Ok(AppsResult::RdpProbeV2 { host, setup })
            }
            AppsCommand::RdpPrepare => {
                self.require_steering(principal)?;
                // The setup may wait for an administrator prompt or a package
                // installation. `Pending` ends the viewer's first-reply wait;
                // no fixed limit follows (user rule r7). QUIC detects a dead peer.
                apps_wire::send(send, &AppsReply::Pending).await?;
                // The device is recorded with the result: after its own
                // request was declined it cannot raise the prompt again (S3).
                let device = format!("{}|{}", principal.device_id, principal.public_key);
                let setup = tokio::task::spawn_blocking(move || {
                    crate::rdp::host_setup::ensure_rdp_host(SetupTrigger::PeerRequest, Some(device))
                })
                .await
                .map_err(eio)?;
                let host = probe_v2().await?;
                self.require_steering(principal)?;
                Ok(AppsResult::RdpPrepare { setup, host })
            }
            AppsCommand::RdpCredential { service_id } => {
                self.require_steering(principal)?;
                let id = service_id.clone();
                let tunnels = self.tcp.registry.clone();
                let issued = tokio::task::spawn_blocking(move || {
                    // A running session to the service's port keeps the
                    // credential it was given (review B8).
                    let in_use = |port: u16| {
                        tunnels.list().iter().any(|tunnel| {
                            tunnel.role == crate::share::tcp_forward::TunnelRole::Target
                                && !tunnel.closing
                                && tunnel.destination.host
                                    == crate::share::access_constraints::TcpHost::Loopback
                                && tunnel.destination.port.get() == port
                        })
                    };
                    crate::rdp::host_setup::issue_managed_credential(&id, in_use)
                })
                .await
                .map_err(eio)?
                .map_err(|reason| io::Error::new(io::ErrorKind::Unsupported, reason))?;
                // A right revoked meanwhile must not receive the fresh secret;
                // the next revocation rotates it again (T-R2).
                if let Err(error) = self.require_steering(principal) {
                    crate::rdp::host_setup::rotate_managed_credentials();
                    return Err(error);
                }
                Ok(AppsResult::RdpCredential {
                    username: issued.0,
                    password: apps_wire::ManagedSecret::new(issued.1),
                })
            }
            AppsCommand::LocalServices => {
                self.require_steering(principal)?;
                let listing = tokio::task::spawn_blocking(local_services)
                    .await
                    .map_err(eio)?;
                self.require_steering(principal)?;
                Ok(match listing {
                    Ok(services) => AppsResult::LocalServices {
                        services,
                        unavailable: None,
                    },
                    Err(reason) => AppsResult::LocalServices {
                        services: Vec::new(),
                        unavailable: Some(reason),
                    },
                })
            }
            _ => Err(eio("device command is no RDP or service command")),
        }
    }

    /// „Bildschirm steuern: Erlauben“ in the proven relation, now.
    pub(in super::super) fn require_steering(&self, principal: &ProvenPrincipal) -> io::Result<()> {
        let state = self
            .auth
            .lock()
            .map_err(|_| eio("authorization unavailable"))?;
        let steers = principal_grant(&state, principal).is_some_and(|grant| {
            crate::share::access_steering::steers(&grant.access_grant(), now_secs())
        });
        if !steers {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Remote Desktop und lokale Dienste brauchen „Bildschirm steuern: Erlauben“",
            ));
        }
        Ok(())
    }
}

/// A fresh probe of this host; it also refreshes the RDP port list, the
/// listener owners and the feed summary.
async fn probe() -> io::Result<crate::rdp::HostProbe> {
    tokio::task::spawn_blocking(crate::rdp::host_setup::refresh_probe)
        .await
        .map_err(eio)
}

/// The probe in the v2 form. A host without any RDP service (Windows Home,
/// Android, a failed inventory) builds its answer with
/// `HostProbe::unavailable`, which is the v1 form; the v2 reply still says
/// v2 so the viewer shows the carried `setup` reason (review B2).
async fn probe_v2() -> io::Result<crate::rdp::HostProbe> {
    let mut host = probe().await?;
    host.version = crate::rdp::core::RDP_PROBE_VERSION_V2;
    Ok(host)
}
