use super::cache_store::CacheStore;
use super::http::DriveHttp;
use super::id_pool::IdPool;
use super::key_locks::{self, KeyGuard, KeyLocks};
use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

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
    /// One folder create per namespace slot (parent ID + encoded name) at a
    /// time, so concurrent transfers never create the same directory twice
    /// (Drive happily makes duplicate same-name folders); other folders are
    /// created in parallel.
    create_slots: Arc<KeyLocks>,
    /// Namespace transitions (rename, promotion) hold the Drive name slots of
    /// their source and destination. Drive names are not unique, so exact-ID
    /// rename verification must not interleave with another in-process rename
    /// or promotion of the same names; unrelated names move in parallel.
    mutation_slots: Arc<KeyLocks>,
    /// Pre-generated ids reserved by uploads that have not reached a locally
    /// verified commit yet. Keeping them across writer retries prevents an
    /// ambiguous completion from allocating a second same-name Drive file.
    pub(super) pending_upload_ids: Arc<Mutex<HashMap<String, String>>>,
    /// Exact pre-generated IDs for metadata-only folder creates whose terminal
    /// result is not yet locally verified. This state is separate from ordinary
    /// path hints: transient validation failure must never discard it.
    pub(super) pending_folder_creates:
        Arc<Mutex<HashMap<String, super::folder_create_journal::PendingFolderCreate>>>,
    /// Private copy stages this backend created (path -> the ID it generated);
    /// only these may be discarded, always by exact ID.
    pub(super) owned_stages: Arc<Mutex<HashMap<String, String>>>,
    pub(super) drive_account_key: Arc<str>,
    pub(super) pending_folder_dir: Option<Arc<PathBuf>>,
    /// Serializes uploads only when they target the same normalized path;
    /// unrelated file uploads remain parallel.
    upload_paths: Arc<KeyLocks>,
    pub(super) id_pool: Arc<IdPool>,
    pub(super) cache_store: Arc<CacheStore>,
    pub(super) binding_store: Arc<super::binding_store::BindingStore>,
    root_id: Arc<Mutex<Option<String>>>,
    pub(super) root: String,
    pub(super) api_base: Arc<str>,
    pub(super) request_timeout: Duration,
    /// Pooled clients: one for API calls and upload sessions, one for
    /// streaming downloads (per-socket inactivity deadlines, no overall
    /// request timeout, so an active large transfer has no wall-clock cap
    /// while a blackholed read still fails).
    pub(super) http: Arc<DriveHttp>,
}

/// Everything a backend is built from.
struct Setup {
    tokens: cloud::Tokens,
    ids: HashMap<String, String>,
    untrusted_ids: HashSet<String>,
    mimes: HashMap<String, String>,
    drive_account_key: String,
    pending_folder_dir: Option<PathBuf>,
    cache_path: Option<PathBuf>,
    binding_dir: Option<PathBuf>,
    root: String,
    api_base: String,
    request_timeout: Duration,
    http: DriveHttp,
}

impl GDriveBackend {
    /// Build from the stored refresh token (must already be connected via
    /// `cloud::authorize`). `root` is the forward-slash start folder.
    pub fn connect(root: &str) -> Result<Self, String> {
        let tokens = cloud::refresh_access(Provider::GDrive)?;
        let http = DriveHttp::new(super::api::DRIVE_REQUEST_TIMEOUT);
        // The account lookup already opens the pooled socket later calls use.
        let drive_account_key = load_drive_account_key(&http.api(), &tokens.access_token)?;
        let cache_path = super::binding_store::account_cache_path(&drive_account_key);
        let loaded = match super::cache::load_from_path(&cache_path) {
            Ok(loaded) => loaded,
            Err(error) if error.kind() == io::ErrorKind::NotFound => super::cache::load(),
            // This is only a hint cache. Durable folder records have their
            // own checked format and must never fall back to empty state.
            Err(_) => super::cache::LoadedCache::default(),
        };
        let mut ids = loaded.ids;
        ids.insert(String::new(), "root".to_string());
        let untrusted_ids = super::cache::loaded_untrusted(&ids);
        Ok(Self::from_setup(Setup {
            tokens,
            ids,
            untrusted_ids,
            mimes: loaded.mimes,
            drive_account_key,
            pending_folder_dir: Some(super::folder_create_journal::record_dir()),
            cache_path: Some(cache_path),
            binding_dir: Some(super::binding_store::record_dir()),
            root: super::core::norm(root),
            api_base: super::api::API.to_string(),
            request_timeout: super::api::DRIVE_REQUEST_TIMEOUT,
            http,
        }))
    }

    fn from_setup(setup: Setup) -> Self {
        let ids = Arc::new(Mutex::new(setup.ids));
        let mimes = Arc::new(Mutex::new(setup.mimes));
        let cache_store = CacheStore::new(setup.cache_path, Arc::clone(&ids), Arc::clone(&mimes));
        GDriveBackend {
            tokens: Arc::new(Mutex::new(setup.tokens)),
            ids,
            untrusted_ids: Arc::new(Mutex::new(setup.untrusted_ids)),
            mimes,
            listed: Arc::new(Mutex::new(HashSet::new())),
            create_slots: KeyLocks::new("Drive-Erzeugungssperre vergiftet"),
            mutation_slots: KeyLocks::new("Drive-Mutationssperre vergiftet"),
            pending_upload_ids: Arc::new(Mutex::new(HashMap::new())),
            pending_folder_creates: Arc::new(Mutex::new(HashMap::new())),
            owned_stages: Arc::new(Mutex::new(HashMap::new())),
            drive_account_key: Arc::from(setup.drive_account_key),
            pending_folder_dir: setup.pending_folder_dir.map(Arc::new),
            upload_paths: KeyLocks::new("Drive-Upload-Pfadsperre vergiftet"),
            id_pool: Arc::new(IdPool::default()),
            cache_store: Arc::new(cache_store),
            binding_store: Arc::new(super::binding_store::BindingStore::new(setup.binding_dir)),
            root_id: Arc::new(Mutex::new(None)),
            root: setup.root,
            api_base: Arc::from(setup.api_base),
            request_timeout: setup.request_timeout,
            http: Arc::new(setup.http),
        }
    }

    pub(super) fn api_url(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.api_base.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }

    pub(super) fn timed_request(&self, request: ureq::Request) -> ureq::Request {
        request.timeout(self.request_timeout)
    }

    #[cfg(test)]
    pub(super) fn test_backend(api_base: &str) -> Self {
        Self::test_backend_with_timeout(api_base, Duration::from_secs(3))
    }

    #[cfg(test)]
    pub(super) fn test_backend_with_timeout(api_base: &str, request_timeout: Duration) -> Self {
        Self::test_backend_with_storage(api_base, request_timeout, None)
    }

    #[cfg(test)]
    pub(super) fn test_backend_with_pending_dir(
        api_base: &str,
        request_timeout: Duration,
        pending_folder_dir: PathBuf,
    ) -> Self {
        Self::test_backend_with_storage(api_base, request_timeout, Some(pending_folder_dir))
    }

    #[cfg(test)]
    fn test_backend_with_storage(
        api_base: &str,
        request_timeout: Duration,
        pending_folder_dir: Option<PathBuf>,
    ) -> Self {
        let binding_dir = pending_folder_dir.as_ref().map(|root| root.join("sync-bindings"));
        Self::test_backend_with_identity(api_base, request_timeout, pending_folder_dir,
            binding_dir, "test-drive-permission-id", "")
    }

    #[cfg(test)]
    pub(super) fn test_backend_with_identity(
        api_base: &str,
        request_timeout: Duration,
        pending_folder_dir: Option<PathBuf>,
        binding_dir: Option<PathBuf>,
        permission_id: &str,
        root: &str,
    ) -> Self {
        let mut ids = HashMap::new();
        ids.insert(String::new(), "root".to_string());
        Self::from_setup(Setup {
            tokens: cloud::Tokens {
                access_token: "test-token".into(),
                refresh_token: "test-refresh".to_string(),
                expires_at: i64::MAX,
            },
            ids,
            untrusted_ids: HashSet::new(),
            mimes: HashMap::new(),
            drive_account_key: super::folder_create_journal::account_key(permission_id),
            pending_folder_dir,
            // Memory-only path cache.
            cache_path: None,
            binding_dir,
            root: super::core::norm(root),
            api_base: api_base.to_string(),
            request_timeout,
            http: DriveHttp::new(request_timeout),
        })
    }

    pub(super) fn tokens_guard(&self) -> io::Result<MutexGuard<'_, cloud::Tokens>> {
        self.tokens
            .lock()
            .map_err(|_| io::Error::other("Drive-Token-Cache vergiftet"))
    }

    pub(super) fn root_id_guard(&self) -> io::Result<MutexGuard<'_, Option<String>>> {
        self.root_id.lock().map_err(|_| io::Error::other("Drive root identity cache poisoned"))
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

    /// Hold the create slot of `segment` (encoded name) below `parent_id`.
    pub(super) fn create_slot_guard(&self, parent_id: &str, segment: &str) -> io::Result<KeyGuard> {
        key_locks::lock(&self.create_slots, &key_locks::slot(parent_id, segment))
    }

    /// Hold the name slots (parent ID, Drive title) a rename or promotion
    /// reads and changes. Callers take their path locks first; slot holders
    /// may create folders (create slots) but never wait for a path lock.
    pub(super) fn mutation_slots_guard(&self, slots: &[(&str, &str)]) -> io::Result<Vec<KeyGuard>> {
        key_locks::lock_all(
            &self.mutation_slots,
            slots
                .iter()
                .map(|(parent_id, title)| key_locks::slot(parent_id, title)),
        )
    }

    pub(super) fn pending_upload_ids_guard(
        &self,
    ) -> io::Result<MutexGuard<'_, HashMap<String, String>>> {
        self.pending_upload_ids
            .lock()
            .map_err(|_| io::Error::other("Drive-Upload-ID-Cache vergiftet"))
    }

    pub(super) fn pending_folder_creates_guard(
        &self,
    ) -> io::Result<
        MutexGuard<'_, HashMap<String, super::folder_create_journal::PendingFolderCreate>>,
    > {
        self.pending_folder_creates
            .lock()
            .map_err(|_| io::Error::other("Drive-Ordnerreservierungs-Cache vergiftet"))
    }

    pub(super) fn owned_stages_guard(&self) -> io::Result<MutexGuard<'_, HashMap<String, String>>> {
        self.owned_stages
            .lock()
            .map_err(|_| io::Error::other("Drive-Stufenliste vergiftet"))
    }

    pub(super) fn upload_path_guard(&self, path: &str) -> io::Result<KeyGuard> {
        key_locks::lock(&self.upload_paths, path)
    }

    /// Lock two paths in lexical order. Transfers take one path lock, while a
    /// rename/promotion takes both; stable ordering prevents reciprocal moves
    /// from deadlocking.
    pub(super) fn upload_path_pair_guard(
        &self,
        left: &str,
        right: &str,
    ) -> io::Result<Vec<KeyGuard>> {
        key_locks::lock_all(&self.upload_paths, [left.to_string(), right.to_string()])
    }
}

/// Drive publishes `user.permissionId` as the requesting user's opaque grantee
/// ID. Unlike a refresh token, it remains the same when OAuth credentials are
/// refreshed or re-authorized, so durable mutation records stay discoverable.
fn load_drive_account_key(agent: &ureq::Agent, access_token: &str) -> Result<String, String> {
    let url = format!("{}/about?fields=user(permissionId)", super::api::API);
    let bearer = format!("Bearer {access_token}");
    let response = agent
        .get(&url)
        .timeout(super::api::DRIVE_REQUEST_TIMEOUT)
        .set("Authorization", &bearer)
        .call()
        .map_err(|error| format!("Drive account identity request failed: {error}"))?;
    let body = response
        .into_string()
        .map_err(|error| format!("Drive account identity response failed: {error}"))?;
    parse_drive_account_key(&body)
}

pub(super) fn parse_drive_account_key(body: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("Drive account identity response is invalid: {error}"))?;
    let permission_id = json["user"]["permissionId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Drive account identity response has no permissionId".to_string())?;
    Ok(super::folder_create_journal::account_key(permission_id))
}

#[cfg(test)]
mod tests {
    use super::parse_drive_account_key;

    #[test]
    fn drive_account_key_uses_stable_permission_id() {
        let first = parse_drive_account_key(
            r#"{"user":{"permissionId":"stable-drive-user"},"ignored":"one"}"#,
        )
        .unwrap();
        let second = parse_drive_account_key(
            r#"{"user":{"permissionId":"stable-drive-user"},"ignored":"two"}"#,
        )
        .unwrap();
        assert_eq!(first, second);
        assert!(parse_drive_account_key(r#"{"user":{}}"#).is_err());
    }
}
