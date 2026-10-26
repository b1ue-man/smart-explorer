//! Owns the exact spawned process handle and the matching daemon TCP lease.
use crate::rdp::core::{AuthenticatedProbe, ClientCapability, HostService, StartRequest};
use crate::share::{
    tcp_forward::{TunnelId, TunnelView},
    PeerOpenTarget,
};
use serde::Serialize;
use std::{
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

#[path = "session_owners.rs"]
mod ownership;
use ownership::{owners, PendingStart};
pub(crate) use ownership::{
    adopt_owned, begin_owned_shutdown, close_owned_session, close_transport, owned_sessions,
    resume_owned_sessions, shutdown_owned_sessions, start_owned,
};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SessionView {
    pub(crate) tunnel: TunnelView,
    pub(crate) client: ClientCapability,
    pub(crate) process_id: u32,
    pub(crate) session_notice: String,
    pub(crate) certificate_sha256: String,
    pub(crate) remote_app: Option<String>,
}

pub(crate) struct RdpSession {
    child: Child,
    _client_resources: Box<dyn Send>,
    view: SessionView,
    tunnel_closed: bool,
    finished: bool,
    registration: Option<PendingStart>,
}
impl RdpSession {
    pub(crate) fn view(&self) -> &SessionView {
        &self.view
    }
    pub(crate) fn tunnel_id(&self) -> TunnelId {
        self.view.tunnel.id.clone()
    }

    /// GUI owns this object; CLI waits with it; a daemon may move it into its
    /// existing job owner. Dropping the owner closes both service and client.
    pub(crate) fn start(
        target: PeerOpenTarget,
        probe: &AuthenticatedProbe,
        request: &StartRequest,
        wait: Duration,
        pending: impl FnMut(),
    ) -> Result<Self, String> {
        Self::start_guarded(target, probe, request, wait, pending, || Ok(()))
    }
    pub(crate) fn start_guarded(
        target: PeerOpenTarget,
        probe: &AuthenticatedProbe,
        request: &StartRequest,
        wait: Duration,
        pending: impl FnMut(),
        admit_client: impl FnOnce() -> Result<(), String>,
    ) -> Result<Self, String> {
        let registration = PendingStart::begin()?;
        if wait.is_zero() || wait > Duration::from_secs(300) {
            return Err("RDP setup wait must be 1..300 seconds".into());
        }
        let service = request.validate(probe)?;
        let client = super::clients::check_client(request)?;
        let tunnel_request = request.tunnel_request(service)?;
        let tunnel = crate::daemon::tcp_start_within(target, tunnel_request, Some(wait), pending)?;
        let closing = { owners().closing };
        let opened = if closing {
            Err("RDP-Start wurde durch das Beenden von Smart Explorer abgebrochen.".into())
        } else {
            admit_client()
                .and_then(|()| Self::on_tunnel(probe, request, service, client, tunnel.clone()))
        };
        match opened {
            Ok(mut session) => {
                session.registration = Some(registration);
                Ok(session)
            }
            Err(error) => match crate::daemon::tcp_close(tunnel.id.clone()) {
                Ok(_) => Err(error),
                Err(close) => {
                    owners()
                        .cleanup
                        .insert(tunnel.id.as_str().into(), tunnel.id.clone());
                    Err(format!(
                        "{error}; TCP lease {} cleanup failed: {close}",
                        tunnel.id.as_str()
                    ))
                }
            },
        }
    }
    fn on_tunnel(
        probe: &AuthenticatedProbe,
        request: &StartRequest,
        service: &HostService,
        client: ClientCapability,
        tunnel: TunnelView,
    ) -> Result<Self, String> {
        // Approval may have waited: recheck the inventory age and every binding.
        request.validate(probe)?;
        probe.check_tunnel(service, &tunnel)?;
        let listener = tunnel
            .listener
            .ok_or_else(|| "RDP listener is missing".to_string())?;
        let certificate = service
            .certificate_sha256
            .as_ref()
            .ok_or_else(|| "RDP fingerprint disappeared from immutable inventory".to_string())?;
        let client_process = super::clients::launch(request, service, listener)?;
        let child = client_process.child;
        let view = SessionView {
            process_id: child.id(),
            tunnel,
            client,
            session_notice: service.session_mode.notice().into(),
            certificate_sha256: certificate.as_str().into(),
            remote_app: request.remote_app.clone(),
        };
        Ok(Self {
            child,
            _client_resources: client_process.resources,
            view,
            tunnel_closed: false,
            finished: false,
            registration: None,
        })
    }
    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| format!("RDP client wait: {error}"))?;
        if status.is_some() {
            self.finished = true;
            self.close_tunnel()?;
            self._client_resources = Box::new(());
        }
        Ok(status)
    }
    pub(crate) fn wait(&mut self) -> Result<ExitStatus, String> {
        let mut last_probe = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if last_probe.elapsed() >= Duration::from_secs(1) {
                match crate::daemon::tcp_status(self.tunnel_id()) {
                    Ok(Some(tunnel)) if !tunnel.closing => {}
                    Ok(_) => {
                        self.close()?;
                        return Err("RDP TCP lease ended or was revoked; client closed".into());
                    }
                    Err(error) => {
                        let close = self.close();
                        return Err(format!(
                            "RDP lease status failed: {error}; cleanup={close:?}"
                        ));
                    }
                }
                last_probe = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn close_tunnel(&mut self) -> Result<(), String> {
        if !self.tunnel_closed {
            crate::daemon::tcp_close(self.tunnel_id())?;
            self.tunnel_closed = true;
        }
        Ok(())
    }
    pub(crate) fn close(&mut self) -> Result<(), String> {
        let tunnel = self.close_tunnel();
        let client = if self.finished {
            Ok(())
        } else {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.finished = true;
                    Ok(())
                }
                Ok(None) => self
                    .child
                    .kill()
                    .and_then(|()| self.child.wait())
                    .map(|_| {
                        self.finished = true;
                    })
                    .map_err(|e| format!("RDP client close: {e}")),
                Err(e) => Err(format!("RDP client close: {e}")),
            }
        };
        match (tunnel, client) {
            (Err(a), Err(b)) => Err(format!("{a}; {b}")),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => {
                self._client_resources = Box::new(());
                Ok(())
            }
        }
    }
}
impl Drop for RdpSession {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
