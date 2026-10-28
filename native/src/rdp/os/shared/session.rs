//! Owns the exact spawned process handle and the matching daemon TCP lease.
//! The tunnel's listener forwards only this client's connections (PID and
//! start time, design 2.7.1) and lives as long as the client and its QUIC
//! connection (design 2.6); the daemon holds its own handle of the process.
use super::clients::Credentials;
use crate::rdp::core::{AuthenticatedProbe, ClientCapability, StartRequest};
use crate::share::{
    tcp_forward::{TunnelId, TunnelView},
    PeerOpenTarget,
};
use serde::Serialize;
use std::{
    net::Ipv4Addr,
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

#[path = "session_owners.rs"]
mod ownership;
pub(crate) use ownership::{
    begin_owned_shutdown, close_owned_session, close_transport, owned_sessions,
    resume_owned_sessions, shutdown_owned_sessions,
};
use ownership::{owners, PendingStart};
#[path = "open_device.rs"]
mod open_device;
#[cfg(any(target_os = "android", all(unix, test)))]
pub(crate) use open_device::open_device;
pub(crate) use open_device::{open_device_session, open_device_with, OpenOptions};
#[cfg(test)]
#[path = "einrichtung_open_task_tests.rs"]
mod einrichtung_open_task_tests;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SessionView {
    pub(crate) tunnel: TunnelView,
    pub(crate) client: ClientCapability,
    pub(crate) process_id: u32,
    pub(crate) session_notice: String,
    pub(crate) certificate_sha256: String,
    pub(crate) remote_app: Option<String>,
}

/// What one start needs besides the probe and the request.
#[derive(Default)]
pub(crate) struct StartContext {
    /// The device address of design 4.3; `None` = 127.0.0.1.
    pub(crate) bind: Option<Ipv4Addr>,
    pub(crate) credentials: Option<Credentials>,
    /// `None` (default): no time limit for the host's answer (design 2.6).
    pub(crate) wait: Option<Duration>,
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
    /// `probe` is consumed: one authenticated probe serves one start.
    pub(crate) fn start(
        target: PeerOpenTarget,
        probe: AuthenticatedProbe,
        request: &StartRequest,
        context: StartContext,
        pending: impl FnMut(),
    ) -> Result<Self, String> {
        Self::start_guarded(target, probe, request, context, pending, || Ok(()))
    }
    pub(crate) fn start_guarded(
        target: PeerOpenTarget,
        probe: AuthenticatedProbe,
        request: &StartRequest,
        context: StartContext,
        pending: impl FnMut(),
        admit_client: impl FnOnce() -> Result<(), String>,
    ) -> Result<Self, String> {
        let registration = PendingStart::begin()?;
        let service = request.validate(&probe)?.clone();
        let client = super::clients::check_client(request)?;
        StartRequest::require_client_evidence(&service, &client)?;
        let credentials = context.credentials.as_ref();
        if credentials.is_none() && super::clients::needs_credential_input(&client.id) {
            return Err(super::clients::NEEDS_SDL.into());
        }
        let tunnel_request = request.tunnel_request(&service)?;
        // Dropping `unbound` on any error below ends its IPC stream; the
        // daemon then closes the tunnel (no half state, design 9.3).
        let unbound = crate::daemon::tcp_start_for_process(
            target,
            tunnel_request,
            context.bind,
            context.wait,
            pending,
        )?;
        if owners().closing {
            return Err("RDP-Start wurde durch das Beenden von Smart Explorer abgebrochen.".into());
        }
        admit_client()?;
        let tunnel = unbound.tunnel().clone();
        probe.check_tunnel(&service, &tunnel)?;
        let listener = tunnel
            .listener
            .ok_or_else(|| "RDP listener is missing".to_string())?;
        let certificate = service
            .certificate_sha256
            .as_ref()
            .ok_or_else(|| "RDP fingerprint disappeared from immutable inventory".to_string())?
            .as_str()
            .to_string();
        let trust = super::clients::prepare_trust(listener, &service, credentials)?;
        let client_process = super::clients::launch(request, &service, listener, credentials)?;
        let mut child = client_process.child;
        let pid = child.id();
        let bound = crate::share::os_sockets::process_identity(pid)
            .map_err(|error| format!("RDP-Client {pid} nicht prüfbar: {error}"))
            .and_then(|identity| unbound.bind(pid, identity.started));
        let tunnel = match bound {
            Ok(tunnel) => tunnel,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{error}; RDP-Client beendet"));
            }
        };
        let view = SessionView {
            process_id: pid,
            tunnel,
            client,
            session_notice: service.session_mode.notice().into(),
            certificate_sha256: certificate,
            remote_app: request.remote_app.clone(),
        };
        Ok(Self {
            child,
            _client_resources: Box::new((client_process.resources, trust)),
            view,
            tunnel_closed: false,
            finished: false,
            registration: Some(registration),
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
