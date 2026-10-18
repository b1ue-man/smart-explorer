//! The screen sessions of this host (SCR-M2, contract V7): what `se screen
//! sessions`, the IPC, the tab „Bildschirm“ and the indicator list; the one
//! session that controls the host at a time (C5: exclusive control,
//! `control_busy`); and the end of every session that a narrowed or ended
//! right hits (`restrict_authorization` with `CapabilityNarrowed` and
//! `GrantExpired`), that the host user stops, or that loses its indicator.
//! A session leaves the registry when its handle goes. Readers wait for a
//! change through a revision (`wait_change`), so the GUI needs no tight
//! polling.
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Notify};

use super::session_state::{EndReason, SessionState};
use super::wire_control::ControlHolder;
use super::wire_hello::SessionMode;
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;

/// Sessions this host runs at once: each costs a capture and an encoder.
pub(crate) const MAX_SESSIONS: usize = 8;
/// Sessions of one device at once: a full view plus previews of its group
/// views.
pub(crate) const MAX_DEVICE_SESSIONS: usize = 4;

/// One session as every surface shows it (`se screen sessions --json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionView {
    pub id: String,
    pub device_id: String,
    pub device_name: String,
    /// `direct` or `room`.
    pub relation: String,
    #[serde(default)]
    pub room_id: Option<String>,
    pub mode: SessionMode,
    pub source: String,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub encoder: Option<String>,
    /// `direct` or `relay`.
    pub path: String,
    /// Unix seconds.
    pub started_at: i64,
    pub bitrate_kbps: u64,
    pub fps: u32,
    pub controlling: bool,
    pub state: SessionState,
}

/// What a new session registers with.
pub(crate) struct NewSession {
    pub principal: ProvenPrincipal,
    pub device_name: String,
    pub mode: SessionMode,
    pub source: String,
    pub path: String,
    pub started_at: i64,
    pub state: SessionState,
    pub screen_content: bool,
}

/// Why a session could not be registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegistryRefusal {
    /// `MAX_SESSIONS` or `MAX_DEVICE_SESSIONS` reached.
    SessionLimit,
}

/// Which sessions `stop` ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StopSelector {
    One(String),
    All,
}

/// Restrictions of a rights change as the registry asks them. Implemented
/// by the share host for its restriction set (`share::node_screen`).
pub(crate) trait SessionRestrictions {
    /// The reason the restrictions end a session of `principal` that needs
    /// `needs`; `None` when they leave it alone.
    fn ends_session(&self, principal: &ProvenPrincipal, needs: &[Capability]) -> Option<EndReason>;
}

struct Entry {
    view: SessionView,
    principal: ProvenPrincipal,
    needs: Vec<Capability>,
    screen_content: bool,
    stop: watch::Sender<Option<EndReason>>,
}

impl Entry {
    /// Asks the session to end; `false` when it was asked before.
    fn signal(&self, reason: EndReason) -> bool {
        if self.stop.borrow().is_some() {
            return false;
        }
        self.stop.send_replace(Some(reason));
        true
    }
}

#[derive(Default)]
struct Inner {
    sessions: BTreeMap<u64, Entry>,
    next_id: u64,
    control: Option<u64>,
    revision: u64,
    epoch: u64,
}

impl Inner {
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn views(&self) -> Vec<SessionView> {
        self.sessions
            .values()
            .map(|entry| entry.view.clone())
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct ScreenRegistry {
    inner: Mutex<Inner>,
    changed: Condvar,
    wake: Notify,
}

impl ScreenRegistry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, mut inner: MutexGuard<'_, Inner>) {
        inner.changed();
        drop(inner);
        self.changed.notify_all();
        self.wake.notify_waiters();
    }

    /// Registers a session; it stays until the handle goes.
    pub(crate) fn register(
        self: &Arc<Self>,
        new: NewSession,
    ) -> Result<SessionHandle, RegistryRefusal> {
        let mut inner = self.lock();
        let device = |entry: &Entry| entry.view.device_id == new.principal.device_id;
        let of_device = inner
            .sessions
            .values()
            .filter(|entry| device(entry))
            .count();
        if inner.sessions.len() >= MAX_SESSIONS || of_device >= MAX_DEVICE_SESSIONS {
            return Err(RegistryRefusal::SessionLimit);
        }
        inner.next_id = inner.next_id.wrapping_add(1);
        let id = inner.next_id;
        let room = new.principal.relation_kind == "room";
        let view = SessionView {
            id: id.to_string(),
            device_id: new.principal.device_id.clone(),
            device_name: new.device_name,
            relation: new.principal.relation_kind.clone(),
            room_id: room.then(|| new.principal.relation_id.clone()),
            mode: new.mode,
            source: new.source,
            codec: None,
            encoder: None,
            path: new.path,
            started_at: new.started_at,
            bitrate_kbps: 0,
            fps: 0,
            controlling: false,
            state: new.state,
        };
        let (stop, receiver) = watch::channel(None);
        let entry = Entry {
            view,
            needs: new.mode.capabilities().to_vec(),
            principal: new.principal,
            screen_content: new.screen_content,
            stop,
        };
        inner.sessions.insert(id, entry);
        self.publish(inner);
        Ok(SessionHandle {
            id,
            registry: self.clone(),
            stop: receiver,
        })
    }

    /// Ends every session a rights change hits (V7: `CapabilityNarrowed`
    /// and `GrantExpired` reach the registry through here). Returns how
    /// many sessions were asked to end.
    pub(crate) fn restrict_authorization(
        &self,
        epoch: u64,
        restrictions: &dyn SessionRestrictions,
    ) -> usize {
        let mut inner = self.lock();
        inner.epoch = inner.epoch.max(epoch);
        let ended = inner
            .sessions
            .values()
            .filter_map(|entry| {
                restrictions
                    .ends_session(&entry.principal, &entry.needs)
                    .map(|reason| entry.signal(reason))
            })
            .filter(|signalled| *signalled)
            .count();
        drop(inner);
        ended
    }

    /// Ends the selected sessions with `reason` (host user, daemon stop).
    pub(crate) fn stop(&self, selector: &StopSelector, reason: EndReason) -> usize {
        let inner = self.lock();
        inner
            .sessions
            .values()
            .filter(|entry| match selector {
                StopSelector::All => true,
                StopSelector::One(id) => &entry.view.id == id,
            })
            .filter(|entry| entry.signal(reason))
            .count()
    }

    /// Ends every session with screen content: its indicator is gone (C7).
    pub(crate) fn end_screen_content(&self, reason: EndReason) -> usize {
        let inner = self.lock();
        inner
            .sessions
            .values()
            .filter(|entry| entry.screen_content && entry.signal(reason))
            .count()
    }

    /// Running sessions with screen content (the indicator stays for them).
    pub(crate) fn screen_content_sessions(&self) -> usize {
        let inner = self.lock();
        inner
            .sessions
            .values()
            .filter(|entry| entry.screen_content)
            .count()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock().sessions.is_empty()
    }

    /// The revision and the sessions, oldest first.
    pub(crate) fn sessions(&self) -> (u64, Vec<SessionView>) {
        let inner = self.lock();
        (inner.revision, inner.views())
    }

    /// Waits until the revision passes `since`, at most `timeout`.
    pub(crate) fn wait_change(&self, since: u64, timeout: Duration) -> (u64, Vec<SessionView>) {
        let inner = self.lock();
        let (inner, _) = self
            .changed
            .wait_timeout_while(inner, timeout, |inner| inner.revision == since)
            .unwrap_or_else(PoisonError::into_inner);
        (inner.revision, inner.views())
    }

    /// Resolves after the next change of the sessions (async readers).
    pub(crate) async fn changed(&self) {
        self.wake.notified().await;
    }

    /// The session that controls the host.
    pub(crate) fn controller(&self) -> Option<ControlHolder> {
        let inner = self.lock();
        let id = inner.control?;
        inner.sessions.get(&id).map(|entry| ControlHolder {
            device_id: entry.view.device_id.clone(),
            device_name: entry.view.device_name.clone(),
        })
    }

    fn take_control(&self, id: u64) -> Result<(), ControlHolder> {
        let mut inner = self.lock();
        if let Some(other) = inner.control.filter(|other| *other != id) {
            if let Some(entry) = inner.sessions.get(&other) {
                return Err(ControlHolder {
                    device_id: entry.view.device_id.clone(),
                    device_name: entry.view.device_name.clone(),
                });
            }
        }
        inner.control = Some(id);
        if let Some(entry) = inner.sessions.get_mut(&id) {
            entry.view.controlling = true;
        }
        self.publish(inner);
        Ok(())
    }

    fn release_control(&self, id: u64) -> bool {
        let mut inner = self.lock();
        if inner.control != Some(id) {
            return false;
        }
        inner.control = None;
        if let Some(entry) = inner.sessions.get_mut(&id) {
            entry.view.controlling = false;
        }
        self.publish(inner);
        true
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut SessionView)) {
        let mut inner = self.lock();
        let Some(entry) = inner.sessions.get_mut(&id) else {
            return;
        };
        let before = entry.view.clone();
        change(&mut entry.view);
        if entry.view != before {
            self.publish(inner);
        }
    }

    fn remove(&self, id: u64) {
        let mut inner = self.lock();
        if inner.control == Some(id) {
            inner.control = None;
        }
        if inner.sessions.remove(&id).is_some() {
            self.publish(inner);
        }
    }
}

/// The registration of one session; dropping it removes the session and
/// releases its control.
pub(crate) struct SessionHandle {
    id: u64,
    registry: Arc<ScreenRegistry>,
    stop: watch::Receiver<Option<EndReason>>,
}

impl SessionHandle {
    #[cfg(test)]
    pub(crate) fn id(&self) -> String {
        self.id.to_string()
    }

    /// A receiver that turns `Some` when the session is asked to end.
    pub(crate) fn stop_signal(&self) -> watch::Receiver<Option<EndReason>> {
        self.stop.clone()
    }

    #[cfg(test)]
    pub(crate) fn stop_reason(&self) -> Option<EndReason> {
        *self.stop.borrow()
    }

    pub(crate) fn update(&self, change: impl FnOnce(&mut SessionView)) {
        self.registry.update(self.id, change);
    }

    /// The session switched between screen content and the test picture.
    pub(crate) fn set_screen_content(&self, screen_content: bool) {
        let mut inner = self.registry.lock();
        if let Some(entry) = inner.sessions.get_mut(&self.id) {
            entry.screen_content = screen_content;
        }
    }

    /// Takes control of the host; the holder when another session has it.
    pub(crate) fn take_control(&self) -> Result<(), ControlHolder> {
        self.registry.take_control(self.id)
    }

    /// Gives control back; `false` when this session did not hold it.
    pub(crate) fn release_control(&self) -> bool {
        self.registry.release_control(self.id)
    }

    pub(crate) fn controller(&self) -> Option<ControlHolder> {
        self.registry.controller()
    }

    /// Whether this session holds the control.
    pub(crate) fn holds_control(&self) -> bool {
        self.registry.lock().control == Some(self.id)
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.registry.remove(self.id);
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
