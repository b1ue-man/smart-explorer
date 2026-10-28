//! The identity a job records for a remote side, when the live backend is
//! reached in a different way than the jobs of that place were recorded
//! with (Z-P2, 2026-10-28). The app reaches a Share device through the
//! worker's proxy (identity `Peer:<label>`); the worker and Android's
//! in-process pool reach the same device through the worker's own
//! `PeerBackend` (identity `peer:<kind>:<relation>:<node>`). A job record,
//! its crash log and its folder aliases keep the app's identity, so a job
//! the worker ran is resumed from the app's recovery list exactly as before,
//! and the reverse. Only the persisted identity changes; every live
//! decision (same namespace, flows) keeps the backend's own.
//!
//! Entries hold a `Weak` of the backend: its allocation cannot be reused
//! while the entry exists, so a pointer match always means this backend.
use crate::vfs::{Backend, BackendHandle};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

struct Alias {
    backend: Weak<dyn Backend>,
    /// The data address of `backend` (thin), compared while it lives.
    address: usize,
    namespace: String,
    state: String,
}

static ALIASES: Mutex<Vec<Alias>> = Mutex::new(Vec::new());

fn aliases() -> MutexGuard<'static, Vec<Alias>> {
    ALIASES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records `namespace` and `state` as what jobs on `backend` persist, for
/// as long as the backend lives. Entries of dropped backends go here.
pub(crate) fn remember(backend: &BackendHandle, namespace: String, state: String) {
    let mut aliases = aliases();
    let address = address_of(&**backend);
    aliases.retain(|alias| alias.backend.strong_count() > 0 && alias.address != address);
    aliases.push(Alias {
        backend: Arc::downgrade(backend),
        address,
        namespace,
        state,
    });
}

/// The thin data address of a backend (`&dyn` and `Arc<dyn>` agree on it).
pub(crate) fn address_of(backend: &dyn Backend) -> usize {
    backend as *const dyn Backend as *const () as usize
}

fn lookup(address: usize) -> Option<(String, String)> {
    aliases()
        .iter()
        .find(|alias| alias.backend.strong_count() > 0 && alias.address == address)
        .map(|alias| (alias.namespace.clone(), alias.state.clone()))
}

/// The recorded identity of `backend` or of the live backend a browsing
/// wrapper (`CachingBackend`) holds; `None` when neither has one.
fn alias_of(backend: &dyn Backend) -> Option<(String, String)> {
    if aliases().is_empty() {
        return None;
    }
    if let Some(found) = lookup(address_of(backend)) {
        return Some(found);
    }
    // Asked without the registry lock: a wrapper may take locks of its own.
    let inner = backend.uncached_backend()?;
    lookup(address_of(&*inner))
}

/// `namespace_identity` as jobs persist it.
pub(crate) fn namespace_of(backend: &dyn Backend) -> String {
    alias_of(backend)
        .map(|(namespace, _)| namespace)
        .unwrap_or_else(|| backend.namespace_identity())
}

/// `namespace_identity` and `state_identity` as jobs persist them.
pub(crate) fn identities_of(backend: &dyn Backend) -> (String, String) {
    alias_of(backend).unwrap_or_else(|| (backend.namespace_identity(), backend.state_identity()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_job_task_alias_names_the_app_identity_and_ends_with_the_backend() {
        let backend: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        let own = (backend.namespace_identity(), backend.state_identity());
        assert_eq!(identities_of(&*backend), own);
        remember(&backend, "Peer:Laptop".into(), "Peer:Laptop".into());
        assert_eq!(
            identities_of(&*backend),
            ("Peer:Laptop".to_string(), "Peer:Laptop".to_string())
        );
        assert_eq!(namespace_of(&*backend), "Peer:Laptop");
        // A browsing wrapper over the same backend persists the same.
        let cached: BackendHandle = Arc::new(crate::vfs::CachingBackend::new(backend.clone()));
        assert_eq!(namespace_of(&*cached), "Peer:Laptop");
        // Another backend keeps its own identity.
        let other: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        assert_eq!(identities_of(&*other), own);
        drop(cached);
        let address = address_of(&*backend);
        drop(backend);
        // The next entry prunes the dead one.
        remember(&other, "Peer:Other".into(), "Peer:Other".into());
        assert!(!aliases().iter().any(|alias| alias.address == address));
        assert_eq!(namespace_of(&*other), "Peer:Other");
    }
}
