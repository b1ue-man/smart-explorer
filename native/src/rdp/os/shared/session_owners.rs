// Central native-client ownership for GUI/MCP/Bridge. Only immutable views
// escape; every process handle remains owned until both child and lease close.
use super::{RdpSession, SessionView, TunnelId};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, PoisonError,
    },
    time::Duration,
};
struct Owned {
    view: SessionView,
    session: Arc<Mutex<RdpSession>>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub(super) struct Owners {
    pub(super) closing: bool,
    pending: usize,
    sessions: BTreeMap<String, Owned>,
    workers: Vec<std::thread::JoinHandle<()>>,
    pub(super) cleanup: BTreeMap<String, TunnelId>,
}
static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
pub(super) fn owners() -> std::sync::MutexGuard<'static, Owners> {
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
pub(super) struct PendingStart;
impl PendingStart {
    pub(super) fn begin() -> Result<Self, String> {
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
