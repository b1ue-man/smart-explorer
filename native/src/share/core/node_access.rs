//! Host boundary for concrete AccessV2 operations. The future app, login and
//! TCP adapters attach real resources through `admit_access`; this module
//! does not advertise or implement any of those protocol consumers.
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use super::access_constraints::{AccessRequest, SessionId, UserId};
use super::access_revocation::{
    AccessConfirmation, AccessHandle, AccessLeaseRegistry, AccessRevocation,
};
use super::core::now_secs;
use super::node::ShareIrohNode;
use super::relation_proof::ProvenPrincipal;
use super::types::ShareAuthState;

/// One host's registry and resource-publication boundary. Lock order is
/// lifecycle -> auth -> registry; policy transitions only need auth -> registry.
/// The callback must never acquire lifecycle/auth again or block on async I/O.
pub(crate) struct NodeAccess {
    registry: Arc<AccessLeaseRegistry>,
    lifecycle: Mutex<()>,
}
impl NodeAccess {
    pub(super) fn new() -> Self {
        Self {
            registry: Arc::new(AccessLeaseRegistry::default()),
            lifecycle: Mutex::new(()),
        }
    }
    pub(crate) fn registry(&self) -> &Arc<AccessLeaseRegistry> {
        &self.registry
    }

    /// `request` contains host-resolved identities. The adapter verifies its
    /// current OS session/user generation inside `admit`, attaches the stop
    /// receiver, and publishes the resource synchronously. It retains the
    /// returned handle through cleanup. No authority survives handle drop.
    pub(crate) fn admit<T>(
        &self,
        auth: &Mutex<ShareAuthState>,
        principal: &ProvenPrincipal,
        request: AccessRequest,
        confirmation: Option<AccessConfirmation>,
        admit: impl FnOnce(&AccessHandle) -> Result<T, AccessRevocation>,
    ) -> Result<(T, AccessHandle), AccessRevocation> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| AccessRevocation::Unavailable)?;
        let state = auth.lock().map_err(|_| AccessRevocation::Unavailable)?;
        let handle = self
            .registry
            .admit(&state, principal, request, confirmation, now_secs())?;
        let resource = admit(&handle)?;
        Ok((resource, handle))
    }
    fn invalidate_session<T>(
        &self,
        auth: &Mutex<ShareAuthState>,
        session: &SessionId,
        publish: impl FnOnce() -> T,
    ) -> Result<(usize, T), AccessRevocation> {
        let _state = auth.lock().map_err(|_| {
            self.registry.close(AccessRevocation::Unavailable);
            AccessRevocation::Unavailable
        })?;
        let revoked = self.registry.invalidate_session(session);
        Ok((revoked, publish()))
    }

    fn admit_existing<T>(
        &self,
        auth: &Mutex<ShareAuthState>,
        handle: &AccessHandle,
        admit: impl FnOnce(&AccessHandle) -> Result<T, AccessRevocation>,
    ) -> Result<T, AccessRevocation> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| AccessRevocation::Unavailable)?;
        let state = auth.lock().map_err(|_| AccessRevocation::Unavailable)?;
        self.registry
            .admit_existing(&state, handle, now_secs(), admit)
    }
    fn invalidate_user<T>(
        &self,
        auth: &Mutex<ShareAuthState>,
        user: &UserId,
        publish: impl FnOnce() -> T,
    ) -> Result<(usize, T), AccessRevocation> {
        let _state = auth.lock().map_err(|_| {
            self.registry.close(AccessRevocation::Unavailable);
            AccessRevocation::Unavailable
        })?;
        let revoked = self.registry.invalidate_user(user);
        Ok((revoked, publish()))
    }
    pub(super) fn stop(&self) {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.registry.close(AccessRevocation::HostStopped);
    }
}
impl Drop for NodeAccess {
    fn drop(&mut self) {
        self.stop();
    }
}

impl ShareIrohNode {
    pub(crate) fn access_registry(&self) -> &Arc<AccessLeaseRegistry> {
        self.access.registry()
    }
    pub(crate) fn admit_access<T>(
        &self,
        principal: &ProvenPrincipal,
        request: AccessRequest,
        confirmation: Option<AccessConfirmation>,
        admit: impl FnOnce(&AccessHandle) -> Result<T, AccessRevocation>,
    ) -> Result<(T, AccessHandle), AccessRevocation> {
        self.access
            .admit(&self.auth, principal, request, confirmation, |handle| {
                self.require_sharing_active()
                    .map_err(|_| AccessRevocation::HostStopped)?;
                admit(handle)
            })
    }
    /// Publish a resource after an authorized async probe without consuming or
    /// copying its Ask receipt again. Keep lifecycle -> auth -> lease -> callback.
    pub(crate) fn admit_existing_access<T>(
        &self,
        handle: &AccessHandle,
        admit: impl FnOnce(&AccessHandle) -> Result<T, AccessRevocation>,
    ) -> Result<T, AccessRevocation> {
        self.access.admit_existing(&self.auth, handle, |handle| {
            self.require_sharing_active()
                .map_err(|_| AccessRevocation::HostStopped)?;
            admit(handle)
        })
    }
    /// Revoke the old session and publish the adapter's replacement facts
    /// inside the same auth boundary as admission. `publish` must not relock
    /// auth. Session IDs include the host generation, never just a recycled
    /// OS numeric ID. Admission checks these facts inside its callback.
    pub(crate) fn invalidate_access_session<T>(
        &self,
        session: &SessionId,
        publish: impl FnOnce() -> T,
    ) -> Result<(usize, T), AccessRevocation> {
        self.access.invalidate_session(&self.auth, session, publish)
    }
    /// Atomically replace the adapter's user facts and revoke that user's
    /// app/login operations. TCP and unrelated users retain their authority.
    pub(crate) fn invalidate_access_user<T>(
        &self,
        user: &UserId,
        publish: impl FnOnce() -> T,
    ) -> Result<(usize, T), AccessRevocation> {
        self.access.invalidate_user(&self.auth, user, publish)
    }
    pub(super) fn start_access_expiry(&self) {
        self.rt.spawn(access_expiry(
            Arc::downgrade(&self.auth),
            self.access_registry().clone(),
        ));
    }
}

/// Started with the node, without any dependency on a screen session.
/// Subscribe before reading deadlines so an admission or moved deadline in
/// between the read and wait remains an unseen watch update.
async fn access_expiry(auth: Weak<Mutex<ShareAuthState>>, registry: Arc<AccessLeaseRegistry>) {
    let mut changed = registry.changes();
    loop {
        let wait = {
            if registry.is_closed() {
                return;
            }
            let Some(auth) = auth.upgrade() else {
                registry.close(AccessRevocation::HostStopped);
                return;
            };
            let Ok(state) = auth.lock() else {
                registry.close(AccessRevocation::Unavailable);
                return;
            };
            let now = now_secs();
            registry.recheck(&state, now);
            let seconds = registry.next_deadline().map_or(30, |end| {
                u64::try_from(end.saturating_sub(now)).unwrap_or(0).min(30)
            });
            Duration::from_secs(seconds)
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {},
            result = changed.changed() => { if result.is_err() { return; } },
        }
    }
}

#[cfg(test)]
#[path = "access_runtime_tests.rs"]
mod tests;
