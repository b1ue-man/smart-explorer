//! Opening the vault locators through the shared resolution (VLT-M3,
//! C2.3; `connect::resolve_endpoint`, Android pool):
//!
//! - `vault://<SITE>/<path>`: the site from the vault list, its unlocked
//!   session in this process, its location resolved like every se location,
//!   and a `VaultBackend` over it. Without a session the vault is locked
//!   (`vault_locked`, C3.2) and nothing is opened.
//! - `vault-store://<site-id>`: the site's location resolved like every se
//!   location, without a wrapper, so content hashes, change signals and the
//!   storage's identity and overlap checks pass through unchanged.
//!
//! Unlocked sessions of this process are published here by the session
//! register (`session.rs`: `publish_session` on unlock, `withdraw_session`
//! and `VaultSession::lock` on lock). A process without the keys of the
//! device (desktop GUI, CLI and MCP beside the worker) opens `vault://`
//! through the worker: `IpcRequest::Vault { req: Open }` serves the worker's
//! `VaultBackend` over the agent protocol (`daemon::open_vault_backend`).
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::vault::core::locator::{vault_location, VaultLocator};
use crate::vault::core::registry_types::SiteRecord;
use crate::vault::core::{VaultSession, VAULT_LOCKED_CODE};
use crate::vfs::BackendHandle;

use super::backend::VaultBackend;
use super::registry_store::RegistryStore;
use super::session::hosts_register;

/// Vaults inside vaults open at most this deep; a site whose location leads
/// back to itself ends here instead of looping.
const MAX_NESTING: u8 = 4;

thread_local! {
    static NESTING: Cell<u8> = const { Cell::new(0) };
}

/// Resolves the location of a site (`connect::resolve_endpoint`).
pub(crate) type ResolveLocation<'a> = &'a dyn Fn(&str) -> Result<(BackendHandle, String), String>;

/// Opens `vault://<site><path>` through the worker that holds the keys
/// (desktop GUI, CLI and MCP; `daemon::open_vault_backend`).
pub(crate) type WorkerOpen<'a> =
    &'a dyn Fn(&SiteRecord, &str) -> Result<(BackendHandle, String), VaultOpenError>;

struct Nesting;

impl Nesting {
    fn enter() -> Result<Self, VaultOpenError> {
        NESTING.with(|depth| {
            if depth.get() >= MAX_NESTING {
                Err(VaultOpenError::Failed(
                    "Tresore sind zu tief ineinander verschachtelt".into(),
                ))
            } else {
                depth.set(depth.get() + 1);
                Ok(Self)
            }
        })
    }
}

impl Drop for Nesting {
    fn drop(&mut self) {
        NESTING.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Why a vault location did not open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VaultOpenError {
    /// The site is entered but not unlocked here (code `vault_locked`).
    Locked { site: String, label: String },
    /// No entered site of this name, or several.
    Unknown(String),
    /// The site's location did not open, or the session does not fit.
    Failed(String),
}

impl VaultOpenError {
    /// The structured code (C3.2), where there is one.
    pub(crate) fn code(&self) -> Option<&'static str> {
        matches!(self, Self::Locked { .. }).then_some(VAULT_LOCKED_CODE)
    }
}

impl fmt::Display for VaultOpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked { site, label } => {
                write!(f, "Tresor {label} ist gesperrt: se vault unlock {site}")
            }
            Self::Unknown(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for VaultOpenError {}

fn locked(site: &SiteRecord) -> VaultOpenError {
    VaultOpenError::Locked {
        site: site.id.clone(),
        label: site.display_label().to_string(),
    }
}

/// An opened `vault://` location.
pub(crate) struct OpenedVault {
    pub(crate) backend: BackendHandle,
    /// The cleartext path inside the vault.
    pub(crate) path: String,
    /// The site ID.
    pub(crate) site: String,
    /// The session behind `backend`: a pool drops the backend once it is
    /// locked, so a new session is used after the next unlock.
    pub(crate) session: Arc<VaultSession>,
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<VaultSession>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<VaultSession>>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

/// Makes `session` the unlocked session of its site in this process.
pub(crate) fn publish_session(session: Arc<VaultSession>) {
    sessions()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session.site().to_string(), session);
}

/// Takes the session of `site` out (the caller locks it).
pub(crate) fn withdraw_session(site: &str) -> Option<Arc<VaultSession>> {
    sessions()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(site)
}

/// The unlocked session of `site` in this process; a locked one is dropped.
pub(crate) fn unlocked_session(site: &str) -> Option<Arc<VaultSession>> {
    let mut sessions = sessions().lock().unwrap_or_else(PoisonError::into_inner);
    match sessions.get(site) {
        Some(session) if !session.is_locked() => Some(Arc::clone(session)),
        Some(_) => {
            sessions.remove(site);
            None
        }
        None => None,
    }
}

/// `vault://<SITE>/<path>` for `connect::resolve_endpoint`: with the
/// session of this process, else through the worker (VLT-M4).
pub(crate) fn open_vault_location(
    site: &str,
    path: &str,
) -> Result<(BackendHandle, String), String> {
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    let worker: WorkerOpen<'_> = &worker_open;
    // A unit test never starts a worker; the worker never asks itself.
    let reach = (!cfg!(test) && !hosts_register()).then_some(worker);
    open_location_in(&RegistryStore::open_default(), site, path, &resolve, reach)
        .map_err(|error| error.to_string())
}

/// `open_vault_location` against a given vault list, resolver and worker:
/// the session of this process first; without one the worker when given,
/// else the site is locked.
pub(crate) fn open_location_in(
    store: &RegistryStore,
    site: &str,
    path: &str,
    resolve: ResolveLocation<'_>,
    worker: Option<WorkerOpen<'_>>,
) -> Result<(BackendHandle, String), VaultOpenError> {
    let record = store.resolve(site).map_err(VaultOpenError::Unknown)?;
    if let Some(session) = unlocked_session(&record.id) {
        return open_with(&record, session, path, resolve)
            .map(|opened| (opened.backend, opened.path));
    }
    match worker {
        Some(worker) => worker(&record, path),
        None => Err(locked(&record)),
    }
}

/// The worker's backend of `record`; its `vault_locked` keeps the code.
fn worker_open(record: &SiteRecord, path: &str) -> Result<(BackendHandle, String), VaultOpenError> {
    crate::daemon::open_vault_backend(&record.id, path).map_err(|error| {
        match error.code.as_deref() {
            Some(VAULT_LOCKED_CODE) => locked(record),
            _ => VaultOpenError::Failed(error.message),
        }
    })
}

/// `vault://<SITE>/<path>` with the structured error, in the process that
/// holds the keys (Android pool).
pub(crate) fn open_vault_backend(site: &str, path: &str) -> Result<OpenedVault, VaultOpenError> {
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    open_in(&RegistryStore::open_default(), site, path, &resolve)
}

/// `open_vault_backend` against a given vault list and location resolver.
pub(crate) fn open_in(
    store: &RegistryStore,
    site: &str,
    path: &str,
    resolve: ResolveLocation<'_>,
) -> Result<OpenedVault, VaultOpenError> {
    let record = store.resolve(site).map_err(VaultOpenError::Unknown)?;
    let session = unlocked_session(&record.id).ok_or_else(|| locked(&record))?;
    open_with(&record, session, path, resolve)
}

/// The `VaultBackend` of an entered site with its unlocked session.
pub(crate) fn open_with(
    site: &SiteRecord,
    session: Arc<VaultSession>,
    path: &str,
    resolve: ResolveLocation<'_>,
) -> Result<OpenedVault, VaultOpenError> {
    if session.is_locked() {
        return Err(locked(site));
    }
    if session.site() != site.id {
        return Err(VaultOpenError::Failed(
            "Die Sitzung gehört zu einem anderen Tresorort".into(),
        ));
    }
    if !site.vault_id.is_empty() && session.config().jti != site.vault_id {
        return Err(VaultOpenError::Failed(
            "Am Speicherort liegt ein anderer Tresor als eingetragen".into(),
        ));
    }
    let _nesting = Nesting::enter()?;
    let (inner, root) = resolve(&site.location).map_err(VaultOpenError::Failed)?;
    let backend = VaultBackend::new(inner, &root, Arc::clone(&session));
    Ok(OpenedVault {
        backend: Arc::new(backend),
        path: path.to_string(),
        site: site.id.clone(),
        session,
    })
}

/// `vault-store://<site-id>` for `connect::resolve_endpoint`.
pub(crate) fn resolve_vault_store(site: &str) -> Result<(BackendHandle, String), String> {
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    resolve_store_in(&RegistryStore::open_default(), site, &resolve)
}

/// `resolve_vault_store` against a given vault list and resolver: the
/// pair (storage backend, vault root) the site's location resolves to.
pub(crate) fn resolve_store_in(
    store: &RegistryStore,
    site: &str,
    resolve: ResolveLocation<'_>,
) -> Result<(BackendHandle, String), String> {
    let record = store
        .get(site)
        .map_err(|error| format!("Tresorliste lesen: {error}"))?
        .ok_or_else(|| format!("Kein Tresorort „{site}“"))?;
    let _nesting = Nesting::enter().map_err(|error| error.to_string())?;
    resolve(&record.location)
}

/// `vault://<site-id><path>` for every accepted spelling of `<SITE>`:
/// stored locators always carry the ID (C2.2).
pub(crate) fn canonical_vault_location(locator: &str) -> Result<String, String> {
    let parsed = VaultLocator::parse(locator)?;
    let site = RegistryStore::open_default().resolve(&parsed.site)?;
    Ok(vault_location(&site.id, &parsed.path))
}

/// (label, location) of every entered site, for the places that name the
/// vaults a changed or removed connection leaves behind (C2.7).
pub(crate) fn site_locations() -> Result<Vec<(String, String)>, String> {
    let load = RegistryStore::open_default()
        .load()
        .map_err(|error| format!("Tresorliste lesen: {error}"))?;
    Ok(load
        .sites
        .iter()
        .map(|site| (site.display_label().to_string(), site.location.clone()))
        .collect())
}
