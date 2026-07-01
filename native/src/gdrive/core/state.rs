use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub struct GDriveBackend {
    pub(super) tokens: Arc<Mutex<cloud::Tokens>>,
    /// path (forward-slash, no trailing slash; "" == root) -> fileId
    pub(super) ids: Arc<Mutex<HashMap<String, String>>>,
    /// Paths loaded from disk must be validated once before they can short-cut
    /// `resolve`; ids learned in this session are not included here.
    pub(super) untrusted_ids: Arc<Mutex<HashSet<String>>>,
    /// path -> mimeType (so we know which files are Google-Docs editors that
    /// must be exported instead of downloaded).
    pub(super) mimes: Arc<Mutex<HashMap<String, String>>>,
    /// Directories whose children are fully known (enumerated by `list_dir`, or
    /// freshly created and therefore empty). For such a parent, a path NOT in
    /// `ids` is known-absent -> we can create it directly and skip the per-file
    /// existence probe. This halves the round-trips during a large first sync.
    pub(super) listed: Arc<Mutex<HashSet<String>>>,
    /// Serializes folder creation so concurrent transfers can't create the same
    /// directory twice (Drive happily makes duplicate same-name folders).
    pub(super) create_lock: Arc<Mutex<()>>,
    pub(super) root: String,
}

impl GDriveBackend {
    /// Build from the stored refresh token (must already be connected via
    /// `cloud::authorize`). `root` is the forward-slash start folder.
    pub fn connect(root: &str) -> Result<Self, String> {
        let tokens = cloud::refresh_access(Provider::GDrive)?;
        let loaded = super::cache::load();
        let mut ids = loaded.ids;
        ids.insert(String::new(), "root".to_string());
        let untrusted_ids = super::cache::loaded_untrusted(&ids);
        Ok(GDriveBackend {
            tokens: Arc::new(Mutex::new(tokens)),
            ids: Arc::new(Mutex::new(ids)),
            untrusted_ids: Arc::new(Mutex::new(untrusted_ids)),
            mimes: Arc::new(Mutex::new(loaded.mimes)),
            listed: Arc::new(Mutex::new(HashSet::new())),
            create_lock: Arc::new(Mutex::new(())),
            root: super::core::norm(root),
        })
    }

    pub(super) fn tokens_guard(&self) -> io::Result<MutexGuard<'_, cloud::Tokens>> {
        self.tokens
            .lock()
            .map_err(|_| io::Error::other("Drive-Token-Cache vergiftet"))
    }

    pub(super) fn ids_guard(&self) -> io::Result<MutexGuard<'_, HashMap<String, String>>> {
        self.ids
            .lock()
            .map_err(|_| io::Error::other("Drive-ID-Cache vergiftet"))
    }

    pub(super) fn untrusted_guard(&self) -> io::Result<MutexGuard<'_, HashSet<String>>> {
        self.untrusted_ids
            .lock()
            .map_err(|_| io::Error::other("Drive-ID-Trust-Cache vergiftet"))
    }

    pub(super) fn mimes_guard(&self) -> io::Result<MutexGuard<'_, HashMap<String, String>>> {
        self.mimes
            .lock()
            .map_err(|_| io::Error::other("Drive-MIME-Cache vergiftet"))
    }

    pub(super) fn listed_guard(&self) -> io::Result<MutexGuard<'_, HashSet<String>>> {
        self.listed
            .lock()
            .map_err(|_| io::Error::other("Drive-Verzeichnisstatus-Cache vergiftet"))
    }

    pub(super) fn create_guard(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.create_lock
            .lock()
            .map_err(|_| io::Error::other("Drive-Erzeugungssperre vergiftet"))
    }
}
