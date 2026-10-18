//! The vault side of vault drives in the worker (contracts V1–V3, S2-V1,
//! S2-V2, S2-V4, S2-V6, S2-V7; C3.3, C3.6; VLT-M12). The drive manager of
//! the worker (`daemon/os/shared/mount_*`) calls it; the drive host never
//! does, it only receives the spool key over IPC.
//!
//! - `open_backend`: the cleartext view of an unlocked site as the source of
//!   a drive (`MountSource::Vault { site_id }`, root `/`); a locked site is
//!   `vault_locked` and nothing is opened.
//! - `spool_key`: the answer to `Vault::SpoolKey`, the drive's spool key
//!   unwrapped (or created) with the site's purpose key `spool-wrap`, only
//!   while the site is unlocked. The masterkey never leaves the worker.
//! - `started`, `ended`: a running drive holds the session as consumer
//!   `Drive(mount id)`; its end (`MountTerminated`) releases it and locks the
//!   vault when nothing else holds the session, unless a lock ended the
//!   drive (C3.3).
//! - `watch`: the drive manager hears of every unlock (restore of the drives
//!   that wait for it, which retries their sealed changes) and of every
//!   lock, which also removes the thumbnails of the vault's drive folders.
//! - Thumbnails (K45, S2-V7, Linux): files inside a drive's folder lose
//!   their freedesktop thumbnails when the drive ends and when its vault
//!   locks.
//!
//! Texts name site IDs and labels only, never a path inside a vault.
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::vault::core::{vault_locked, VAULT_LOCKED_CODE};
use crate::vfs::BackendHandle;

use super::open::{open_with, VaultOpenError};
use super::registry_store::RegistryStore;
use super::session::{
    register, ConsumerGuard, ConsumerKind, RegisterEvent, SessionRegister, SiteState,
};
use super::spool_key::{self, SpoolKey};

/// Resolves the location of a site (`connect::resolve_endpoint`).
pub(crate) type Resolver =
    Box<dyn Fn(&str) -> Result<(BackendHandle, String), String> + Send + Sync>;
/// Told the site ID of every unlock, on a thread of its own.
pub(crate) type UnlockHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Why a vault drive has no source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MountVaultError {
    /// The site is entered but locked (code `vault_locked`).
    Locked { site: String, label: String },
    /// No such site, or its location did not open.
    Failed(String),
}

impl MountVaultError {
    /// The structured code (C3.2), where there is one.
    pub(crate) fn code(&self) -> Option<&'static str> {
        matches!(self, Self::Locked { .. }).then_some(VAULT_LOCKED_CODE)
    }
}

impl fmt::Display for MountVaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked { site, label } => {
                write!(f, "Tresor {label} ist gesperrt: se vault unlock {site}")
            }
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for MountVaultError {}

/// A running vault drive as the bridge knows it.
struct Drive {
    site: String,
    place: Option<PathBuf>,
    _consumer: ConsumerGuard,
}

/// The vault side of the drives of one register.
pub(crate) struct VaultDriveBridge {
    register: Arc<SessionRegister>,
    store: RegistryStore,
    resolve: Resolver,
    /// Thumbnail cache to clean; `None` is the user's cache.
    thumbnails: Option<PathBuf>,
    drives: Mutex<HashMap<String, Drive>>,
    watching: AtomicBool,
}

impl VaultDriveBridge {
    pub(crate) fn new(
        register: Arc<SessionRegister>,
        store: RegistryStore,
        resolve: Resolver,
        thumbnails: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            register,
            store,
            resolve,
            thumbnails,
            drives: Mutex::new(HashMap::new()),
            watching: AtomicBool::new(false),
        })
    }

    fn drives(&self) -> MutexGuard<'_, HashMap<String, Drive>> {
        self.drives.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The cleartext view of the unlocked site `site` (its ID).
    pub(crate) fn open_backend(&self, site: &str) -> Result<BackendHandle, MountVaultError> {
        let record = match self.store.get(site) {
            Ok(Some(record)) => record,
            Ok(None) => return Err(MountVaultError::Failed(format!("Kein Tresorort „{site}“"))),
            Err(error) => {
                return Err(MountVaultError::Failed(format!(
                    "Tresorliste lesen: {error}"
                )))
            }
        };
        let locked = || MountVaultError::Locked {
            site: record.id.clone(),
            label: record.display_label().to_string(),
        };
        let Some((session, _)) = self.register.session(&record.id) else {
            return Err(locked());
        };
        match open_with(&record, session, "/", &*self.resolve) {
            Ok(opened) => Ok(opened.backend),
            Err(VaultOpenError::Locked { .. }) => Err(locked()),
            Err(error) => Err(MountVaultError::Failed(error.to_string())),
        }
    }

    /// The spool key of the drive whose cache folder is `mount_dir`; a
    /// locked site is `vault_locked` and no key file is written.
    pub(crate) fn spool_key(&self, site: &str, mount_dir: &Path) -> io::Result<SpoolKey> {
        self.register
            .with_spool_wrap_key(site, |wrap| spool_key::load_or_create(mount_dir, wrap))?
    }

    /// A drive of `site` runs: it holds the session until it ends.
    pub(crate) fn started(
        &self,
        site: &str,
        mount_id: &str,
        place: Option<PathBuf>,
    ) -> io::Result<()> {
        let consumer = self
            .register
            .consume(site, ConsumerKind::Drive(mount_id.to_string()))
            .ok_or_else(|| vault_locked(site))?;
        let replaced = self.drives().insert(
            mount_id.to_string(),
            Drive {
                site: site.to_string(),
                place,
                _consumer: consumer,
            },
        );
        drop(replaced);
        Ok(())
    }

    /// A drive of `site` ended (`MountTerminated`): its consumer goes, its
    /// thumbnails go, and unless a lock ended it the vault locks when
    /// nothing else holds the session. Returns whether the vault locked.
    pub(crate) fn ended(
        &self,
        site: &str,
        mount_id: &str,
        place: Option<&Path>,
        by_lock: bool,
    ) -> bool {
        let known = self.drives().remove(mount_id);
        let place = place
            .map(Path::to_path_buf)
            .or_else(|| known.as_ref().and_then(|drive| drive.place.clone()));
        // The consumer is released outside the bridge's own lock.
        drop(known);
        if let Some(place) = place {
            self.remove_thumbnails(&place);
        }
        self.register.mount_ended(site, mount_id, by_lock)
    }

    /// Watches the register once: `on_unlock` hears of every unlock (also
    /// of a refused lock, which leaves the site unlocked); a lock removes
    /// the thumbnails of the site's drive folders.
    pub(crate) fn watch(self: &Arc<Self>, on_unlock: UnlockHook) {
        if self.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        let bridge = Arc::downgrade(self);
        self.register
            .subscribe(Arc::new(move |event: &RegisterEvent| {
                let Some(bridge) = bridge.upgrade() else {
                    return;
                };
                match event.state {
                    SiteState::Unlocked => {
                        // Listeners run inside unlock and lock calls; starting
                        // drives must never wait for them.
                        let hook = Arc::clone(&on_unlock);
                        let site = event.site.clone();
                        let _ = std::thread::Builder::new()
                            .name("vault-drive-unlock".into())
                            .spawn(move || hook(&site));
                    }
                    SiteState::Locked => bridge.locked(&event.site),
                    SiteState::Locking | SiteState::Unavailable => {}
                }
            }));
    }

    fn locked(&self, site: &str) {
        let places: Vec<PathBuf> = self
            .drives()
            .values()
            .filter(|drive| drive.site == site)
            .filter_map(|drive| drive.place.clone())
            .collect();
        for place in places {
            self.remove_thumbnails(&place);
        }
    }

    fn remove_thumbnails(&self, place: &Path) {
        super::remove_drive_thumbnails(self.thumbnails.as_deref(), place);
    }
}

/// The bridge of this process: the session register, the vault list and
/// the shared location resolution.
pub(crate) fn vault_drive_bridge() -> &'static Arc<VaultDriveBridge> {
    static BRIDGE: OnceLock<Arc<VaultDriveBridge>> = OnceLock::new();
    BRIDGE.get_or_init(|| {
        VaultDriveBridge::new(
            Arc::clone(register()),
            RegistryStore::open_default(),
            Box::new(|location: &str| crate::connect::resolve_endpoint(location)),
            None,
        )
    })
}
