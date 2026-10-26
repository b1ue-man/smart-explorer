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

// Central native-client ownership for GUI/MCP/Bridge. Only immutable views
// escape; every process handle remains owned until both child and lease close.
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, PoisonError,
    },
};
struct Owned {
    view: SessionView,
    session: Arc<Mutex<RdpSession>>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct Owners {
    closing: bool,
    pending: usize,
    sessions: BTreeMap<String, Owned>,
    workers: Vec<std::thread::JoinHandle<()>>,
    cleanup: BTreeMap<String, TunnelId>,
}
static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
fn owners() -> std::sync::MutexGuard<'static, Owners> {
    OWNERS
        .get_or_init(|| Mutex::new(Owners::default()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}
pub(crate) fn owned_sessions() -> Vec<SessionView> {
    owners()
        .sessions
        .values()
        .map(|entry| entry.view.clone())
        .collect()
}
/// Latch admission before the GUI starts its drain worker. This holds only the
/// registry lock: no daemon IPC or child wait may block the window thread.
pub(crate) fn begin_owned_shutdown() -> bool {
    let mut state = owners();
    state.closing = true;
    for entry in state.sessions.values_mut() {
        entry.cancel.store(true, Ordering::Release);
        entry.view.tunnel.closing = true;
    }
    state.pending > 0
        || !state.sessions.is_empty()
        || !state.cleanup.is_empty()
        || !state.workers.is_empty()
}
/// A rejected/cancelled GUI exit may admit sessions again only after its drain
/// joined every monitor and removed every child, pending start and lease.
pub(crate) fn resume_owned_sessions() -> Result<(), String> {
    let mut state = owners();
    if state.pending > 0
        || !state.sessions.is_empty()
        || !state.cleanup.is_empty()
        || !state.workers.is_empty()
    {
        return Err(
            "RDP-Sitzungen werden noch beendet; neue Starts bleiben bis zum Abschluss gesperrt."
                .into(),
        );
    }
    state.closing = false;
    Ok(())
}
pub(crate) fn close_owned_session(id: &TunnelId) -> Result<bool, String> {
    let session = {
        let mut state = owners();
        let Some(entry) = state.sessions.get_mut(id.as_str()) else {
            return Ok(false);
        };
        entry.cancel.store(true, Ordering::Release);
        entry.view.tunnel.closing = true;
        entry.session.clone()
    };
    let result = session
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .close();
    if result.is_ok() {
        owners().sessions.remove(id.as_str());
    }
    result.map(|()| true)
}
pub(crate) fn close_transport(id: TunnelId) -> Result<bool, String> {
    let closed = crate::daemon::tcp_close(id.clone())?;
    owners().cleanup.remove(id.as_str());
    Ok(closed)
}
struct PendingStart;
impl PendingStart {
    fn begin() -> Result<Self, String> {
        // Retry one retained failed-setup lease before allocating a new slot.
        // This also removes a lease already closed through the shared TCP UI.
        let cleanup = {
            let state = owners();
            if state.closing {
                return Err(
                    "Smart Explorer beendet die RDP-Sitzungen; ein neuer Start ist gesperrt."
                        .into(),
                );
            }
            state.cleanup.values().next().cloned()
        };
        if let Some(id) = cleanup {
            let _ = close_transport(id);
        }
        let mut state = owners();
        if state.closing {
            return Err(
                "Smart Explorer beendet die RDP-Sitzungen; ein neuer Start ist gesperrt.".into(),
            );
        }
        let active_workers = state
            .workers
            .iter()
            .filter(|worker| !worker.is_finished())
            .count();
        if state.sessions.len().max(active_workers) + state.pending + state.cleanup.len()
            >= crate::share::tcp_forward::MAX_TCP_TUNNELS
        {
            return Err("Die vorhandene TCP-Sitzungsgrenze ist erreicht.".into());
        }
        state.pending += 1;
        Ok(Self)
    }
}
impl Drop for PendingStart {
    fn drop(&mut self) {
        let mut state = owners();
        state.pending = state.pending.saturating_sub(1);
    }
}
pub(crate) fn start_owned(
    target: PeerOpenTarget,
    probe: &AuthenticatedProbe,
    request: &StartRequest,
    wait: Duration,
    pending: impl FnMut(),
) -> Result<SessionView, String> {
    let session = RdpSession::start_guarded(target, probe, request, wait, pending, || {
        if owners().closing {
            Err("RDP-Start wurde durch das Beenden von Smart Explorer abgebrochen.".into())
        } else {
            Ok(())
        }
    })?;
    adopt_owned(session)
}
pub(crate) fn adopt_owned(mut session: RdpSession) -> Result<SessionView, String> {
    // This guard spans the return from start(), registry insertion and worker
    // registration. Shutdown cannot observe a gap with a live unregistered child.
    let _registration = session.registration.take();
    let view = session.view().clone();
    let key = view.tunnel.id.as_str().to_string();
    let session = Arc::new(Mutex::new(session));
    let cancel = Arc::new(AtomicBool::new(false));
    let closing = {
        let mut state = owners();
        if state.sessions.len() >= crate::share::tcp_forward::MAX_TCP_TUNNELS
            || state.sessions.contains_key(&key)
        {
            return Err(
                "Der RDP-Sitzungsbesitzer ist voll oder die Sitzung ist bereits registriert."
                    .into(),
            );
        }
        cancel.store(state.closing, Ordering::Release);
        let closing = state.closing;
        state.sessions.insert(
            key.clone(),
            Owned {
                view: view.clone(),
                session: session.clone(),
                cancel: cancel.clone(),
            },
        );
        closing
    };
    let failed_key = key.clone();
    let worker_id = view.tunnel.id.clone();
    let worker = std::thread::Builder::new()
        .name("rdp-native-owner".into())
        .spawn(move || loop {
            if cancel.load(Ordering::Acquire) {
                let result = session
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .close();
                if result.is_ok() {
                    owners().sessions.remove(&key);
                    break;
                }
                if let Some(entry) = owners().sessions.get_mut(&key) {
                    entry.view.tunnel.last_error = result.err();
                }
            } else {
                let finished = session
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .try_wait();
                match finished {
                    Ok(Some(_)) => {
                        owners().sessions.remove(&key);
                        break;
                    }
                    Err(error) => {
                        cancel.store(true, Ordering::Release);
                        if let Some(entry) = owners().sessions.get_mut(&key) {
                            entry.view.tunnel.last_error = Some(error);
                        }
                    }
                    Ok(None) => match crate::daemon::tcp_status(worker_id.clone()) {
                        Ok(Some(tunnel)) if !tunnel.closing => {
                            if let Some(entry) = owners().sessions.get_mut(&key) {
                                entry.view.tunnel = tunnel;
                            }
                        }
                        result => {
                            cancel.store(true, Ordering::Release);
                            if let Some(entry) = owners().sessions.get_mut(&key) {
                                entry.view.tunnel.closing = true;
                                entry.view.tunnel.last_error = Some(match result {
                                    Err(error) => error,
                                    _ => "Die TCP-Lease wurde beendet oder entzogen.".into(),
                                });
                            }
                        }
                    },
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        });
    match worker {
        Ok(worker) => {
            let mut state = owners();
            // Completed monitors hold no native child/profile. Reap them at
            // each adoption so the join-handle list cannot grow without bound.
            let mut live = Vec::new();
            for previous in state.workers.drain(..) {
                if previous.is_finished() {
                    let _ = previous.join();
                } else {
                    live.push(previous);
                }
            }
            live.push(worker);
            state.workers = live;
        }
        Err(error) => {
            let id = TunnelId::parse(&failed_key)?;
            let cleanup = close_owned_session(&id);
            return Err(format!("Der RDP-Sitzungsbesitzer konnte nicht gestartet werden: {error}; Aufräumen: {cleanup:?}"));
        }
    }
    if closing {
        Err("RDP-Start wird wegen des Beendens von Smart Explorer geschlossen.".into())
    } else {
        Ok(view)
    }
}
/// Main calls this during normal app/MCP shutdown, after its existing busy
/// gates and before it marks shutdown prepared. Retry on Err; never exit while
/// a pending start or an owned client/lease still needs cleanup.
pub(crate) fn shutdown_owned_sessions() -> Result<(), String> {
    let (pending, ids, cleanup) = {
        let mut state = owners();
        state.closing = true;
        (
            state.pending,
            state
                .sessions
                .values()
                .map(|entry| entry.view.tunnel.id.clone())
                .collect::<Vec<_>>(),
            state.cleanup.values().cloned().collect::<Vec<_>>(),
        )
    };
    let mut errors = Vec::new();
    for id in ids {
        if let Err(error) = close_owned_session(&id) {
            errors.push(error);
        }
    }
    for id in cleanup {
        match crate::daemon::tcp_close(id.clone()) {
            Ok(_) => {
                owners().cleanup.remove(id.as_str());
            }
            Err(error) => errors.push(format!(
                "RDP-TCP-Lease {} konnte noch nicht aufgeräumt werden: {error}",
                id.as_str()
            )),
        }
    }
    if pending > 0 {
        errors.push("Ein laufender RDP-Start wird abgebrochen und aufgeräumt. Das Fenster bleibt bis zum Abschluss offen.".into());
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let workers = {
        let mut state = owners();
        if state.pending > 0
            || !state.sessions.is_empty()
            || !state.cleanup.is_empty()
            || state.workers.iter().any(|worker| !worker.is_finished())
        {
            return Err("RDP-Sitzungsbesitzer werden noch beendet. Das Fenster bleibt bis zum Abschluss offen.".into());
        }
        std::mem::take(&mut state.workers)
    };
    for worker in workers {
        if worker.join().is_err() {
            return Err("Ein RDP-Sitzungsbesitzer wurde unerwartet beendet; seine Ressourcen wurden zuvor geschlossen.".into());
        }
    }
    Ok(())
}
