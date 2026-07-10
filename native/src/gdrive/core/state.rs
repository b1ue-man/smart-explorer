use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

struct UploadPathLocks {
    active: Mutex<HashSet<String>>,
    ready: Condvar,
}

pub(super) struct UploadPathGuard {
    locks: Arc<UploadPathLocks>,
    path: String,
}

pub(super) struct UploadPathPairGuard {
    _first: UploadPathGuard,
    _second: Option<UploadPathGuard>,
}

impl Drop for UploadPathGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.locks.active.lock() {
            active.remove(&self.path);
            self.locks.ready.notify_all();
        }
    }
}

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
    /// freshly created and therefore empty). Folder creation can use this to
    /// skip a redundant lookup; file uploads always re-probe because Drive
    /// sibling names are not unique and this snapshot can become stale.
    pub(super) listed: Arc<Mutex<HashSet<String>>>,
    /// Serializes folder creation so concurrent transfers can't create the same
    /// directory twice (Drive happily makes duplicate same-name folders).
    pub(super) create_lock: Arc<Mutex<()>>,
    /// Serializes namespace transitions that span more than one path. Drive
    /// names are not unique, so exact-ID rename verification must not interleave
    /// with another in-process rename or promotion.
    pub(super) mutation_lock: Arc<Mutex<()>>,
    /// Pre-generated ids reserved by uploads that have not reached a locally
    /// verified commit yet. Keeping them across writer retries prevents an
    /// ambiguous completion from allocating a second same-name Drive file.
    pub(super) pending_upload_ids: Arc<Mutex<HashMap<String, String>>>,
    /// Serializes uploads only when they target the same normalized path;
    /// unrelated file uploads remain parallel.
    upload_paths: Arc<UploadPathLocks>,
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
            mutation_lock: Arc::new(Mutex::new(())),
            pending_upload_ids: Arc::new(Mutex::new(HashMap::new())),
            upload_paths: Arc::new(UploadPathLocks {
                active: Mutex::new(HashSet::new()),
                ready: Condvar::new(),
            }),
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

    pub(super) fn mutation_guard(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.mutation_lock
            .lock()
            .map_err(|_| io::Error::other("Drive-Mutationssperre vergiftet"))
    }

    pub(super) fn pending_upload_ids_guard(
        &self,
    ) -> io::Result<MutexGuard<'_, HashMap<String, String>>> {
        self.pending_upload_ids
            .lock()
            .map_err(|_| io::Error::other("Drive-Upload-ID-Cache vergiftet"))
    }

    pub(super) fn upload_path_guard(&self, path: &str) -> io::Result<UploadPathGuard> {
        let mut active = self
            .upload_paths
            .active
            .lock()
            .map_err(|_| io::Error::other("Drive-Upload-Pfadsperre vergiftet"))?;
        while active.contains(path) {
            active = self
                .upload_paths
                .ready
                .wait(active)
                .map_err(|_| io::Error::other("Drive-Upload-Pfadsperre vergiftet"))?;
        }
        active.insert(path.to_string());
        Ok(UploadPathGuard {
            locks: self.upload_paths.clone(),
            path: path.to_string(),
        })
    }

    /// Lock two paths in lexical order. Transfers take one path lock, while a
    /// rename/promotion takes both; stable ordering prevents reciprocal moves
    /// from deadlocking.
    pub(super) fn upload_path_pair_guard(
        &self,
        left: &str,
        right: &str,
    ) -> io::Result<UploadPathPairGuard> {
        let (first, second) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let first = self.upload_path_guard(first)?;
        let second_guard = if first.path == second {
            None
        } else {
            Some(self.upload_path_guard(second)?)
        };
        Ok(UploadPathPairGuard {
            _first: first,
            _second: second_guard,
        })
    }
}
