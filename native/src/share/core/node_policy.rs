//! Live authority survives transport loss, but never a matching restriction.
use std::{collections::HashMap, io, sync::{Arc, Mutex, Weak}};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use iroh::endpoint::Connection;
use super::{relation_rights::RestrictionSet, session::{PeerDeviceKey, PeerPrincipal}};

struct Scope {
    principal: PeerPrincipal,
    revision: AtomicU64,
}

#[derive(Clone)]
pub(super) struct Generation {
    scope: Arc<Scope>,
    revision: u64,
}

impl Generation {
    pub(super) fn check(&self) -> io::Result<()> {
        if self.scope.revision.load(Ordering::Acquire) == self.revision { Ok(()) }
        else { Err(io::Error::new(io::ErrorKind::PermissionDenied, "Share-Autorisierung wurde entzogen")) }
    }
    pub(super) fn revision(&self) -> u64 { self.revision }
    pub(super) fn principal(&self) -> &PeerPrincipal { &self.scope.principal }
}

struct Binding { connection: Connection, generation: Generation, _admission: Option<super::fair_admission::Permit<PeerDeviceKey>> }
#[derive(Default)]
struct State {
    scopes: HashMap<PeerPrincipal, Weak<Scope>>,
    connections: HashMap<usize, Binding>,
    cancels: Vec<(Generation, Weak<AtomicBool>)>,
}

pub(super) struct SessionPolicy {
    state: Mutex<State>,
    incoming: Arc<super::fair_admission::Pool<PeerDeviceKey>>,
}
impl Default for SessionPolicy {
    fn default() -> Self { Self { state: Mutex::new(State::default()), incoming: super::fair_admission::Pool::new(64) } }
}

impl SessionPolicy {
    pub(super) fn snapshot(&self, principal: PeerPrincipal) -> io::Result<Generation> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Share-Autorisierung gesperrt"))?;
        state.scopes.retain(|_, scope| scope.strong_count() > 0);
        let scope = state.scopes.get(&principal).and_then(Weak::upgrade).unwrap_or_else(|| {
            let scope = Arc::new(Scope { principal: principal.clone(), revision: AtomicU64::new(0) });
            state.scopes.insert(principal, Arc::downgrade(&scope));
            scope
        });
        let revision = scope.revision.load(Ordering::Acquire);
        Ok(Generation { scope, revision })
    }

    pub(super) fn bind(&self, connection: &Connection, generation: Generation) -> io::Result<()> {
        self.bind_admitted(connection, generation, None)
    }
    pub(super) fn connection_ticket(&self, principal: &PeerPrincipal) -> io::Result<super::fair_admission::Ticket<PeerDeviceKey>> {
        self.incoming.enqueue(principal.device_identity())
    }
    pub(super) fn bind_admitted(&self, connection: &Connection, generation: Generation,
        admission: Option<super::fair_admission::Permit<PeerDeviceKey>>) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Share-Sitzungen gesperrt"))?;
        generation.check()?;
        state.connections.retain(|_, binding| binding.connection.close_reason().is_none());
        state.connections.insert(connection.stable_id(), Binding { connection: connection.clone(), generation, _admission: admission });
        Ok(())
    }
    pub(super) fn unbind(&self, id: usize) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).connections.remove(&id);
    }
    pub(super) fn borrowed_connection(&self, candidates: &[usize], waiting: &PeerPrincipal) -> io::Result<Option<usize>> {
        let state = self.state.lock().map_err(|_| io::Error::other("Share-Sitzungen gesperrt"))?;
        let mut counts = HashMap::<PeerDeviceKey, usize>::new();
        for binding in state.connections.values().filter(|binding| binding._admission.is_some()) {
            *counts.entry(binding.generation.principal().device_identity()).or_default() += 1;
        }
        let device = waiting.device_identity();
        let owned = counts.get(&device).copied().unwrap_or(0);
        Ok(candidates.iter().filter_map(|id| {
            let binding = state.connections.get(id)?;
            let peer = binding.generation.principal().device_identity();
            let count = counts.get(&peer).copied().unwrap_or(0);
            (peer != device && count > owned.saturating_add(1)).then_some((*id, count))
        }).max_by_key(|(_, count)| *count).map(|(id, _)| id))
    }

    pub(super) fn register_cancel(&self, generation: &Generation, cancel: &Arc<AtomicBool>) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Share-Arbeit gesperrt"))?;
        if let Err(error) = generation.check() {
            cancel.store(true, Ordering::Release);
            return Err(error);
        }
        state.cancels.retain(|(_, marker)| marker.strong_count() > 0);
        state.cancels.push((generation.clone(), Arc::downgrade(cancel)));
        Ok(())
    }

    pub(super) fn invalidate(&self, restrictions: &RestrictionSet, revision: u64) -> io::Result<Vec<Connection>> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Share-Autorisierung gesperrt"))?;
        for scope in state.scopes.values().filter_map(Weak::upgrade) {
            if scope.principal.affected_by(restrictions) { scope.revision.store(revision, Ordering::Release); }
        }
        state.cancels.retain(|(generation, marker)| {
            let Some(cancel) = marker.upgrade() else { return false };
            if generation.principal().affected_by(restrictions) { cancel.store(true, Ordering::Release); }
            true
        });
        let ids: Vec<_> = state.connections.iter().filter(|(_, binding)|
            binding.generation.principal().affected_by(restrictions)).map(|(id, _)| *id).collect();
        Ok(ids.into_iter().filter_map(|id| state.connections.remove(&id).map(|b| b.connection)).collect())
    }

    pub(super) fn is_bound(&self, connection: &Connection) -> io::Result<bool> {
        Ok(self.state.lock().map_err(|_| io::Error::other("Share-Sitzungen gesperrt"))?
            .connections.contains_key(&connection.stable_id()))
    }
}

#[cfg(test)]
#[path = "node_policy_task_tests.rs"]
mod task_tests;
