//! Session authority is independent of a physical QUIC connection.
use std::{io, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};
use crate::share::{core::eio, node::ShareIrohNode, node_policy::Generation,
    relation_rights::SessionAuthorization, session::IncomingSession, types::ShareAuthState};

pub(in crate::share) struct AccessAuthority {
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<ShareAuthState>>,
    node: Weak<ShareIrohNode>,
    generation: Generation,
    transport: Option<Arc<AtomicBool>>,
}

pub(in crate::share) struct StreamGuard(Arc<AtomicBool>);
impl Drop for StreamGuard {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}

impl AccessAuthority {
    pub(in crate::share) fn new(session: Arc<IncomingSession>, auth: Arc<Mutex<ShareAuthState>>,
        node: &Arc<ShareIrohNode>) -> io::Result<(Self, SessionAuthorization)> {
        node.require_sharing_active()?;
        // Configuration publishes restrictions under this same lock. Couple
        // the exports to their generation, never an old root to a new grant.
        let state = auth.lock().map_err(|_| eio("Share-Auth gesperrt"))?;
        let rights = session.authorize_state(&state)?;
        let generation = node.policy.snapshot(session.principal())?;
        drop(state);
        Ok((Self { session, auth, node: Arc::downgrade(node), generation,
            transport: Some(Arc::new(AtomicBool::new(false))) }, rights))
    }
    pub(in crate::share) fn check(&self) -> io::Result<SessionAuthorization> {
        self.node.upgrade().ok_or_else(|| eio("Share ist gestoppt"))?.require_sharing_active()?;
        if self.transport.as_ref().is_some_and(|gone| gone.load(Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Share-Anfrage wurde beendet"));
        }
        self.generation.check()?;
        self.session.authorize(&self.auth).map_err(|_| io::Error::new(
            io::ErrorKind::PermissionDenied, "Share-Anfrage nicht autorisiert"))
    }
    pub(in crate::share) fn check_write(&self) -> io::Result<()> {
        if self.check()?.may_write { Ok(()) }
        else { Err(io::Error::new(io::ErrorKind::ReadOnlyFilesystem, "Share-Beziehung ist nur lesbar")) }
    }
    pub(in crate::share) fn revision(&self) -> u64 { self.generation.revision() }
    pub(in crate::share) fn stream_guard(&self) -> Option<StreamGuard> {
        self.transport.as_ref().map(|marker| StreamGuard(marker.clone()))
    }
    pub(in crate::share) fn retained(&self) -> Self {
        Self { session: self.session.clone(), auth: self.auth.clone(), node: self.node.clone(),
            generation: self.generation.clone(), transport: None }
    }
    pub(in crate::share) fn register_cancel(&self, cancel: &Arc<AtomicBool>) -> io::Result<()> {
        self.check()?;
        self.node.upgrade().ok_or_else(|| eio("Share ist gestoppt"))?
            .policy.register_cancel(&self.generation, cancel)
    }
}
