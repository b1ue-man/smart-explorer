use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

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
    /// Exact pre-generated IDs for metadata-only folder creates whose terminal
    /// result is not yet locally verified. This state is separate from ordinary
    /// path hints: transient validation failure must never discard it.
    pub(super) pending_folder_creates:
        Arc<Mutex<HashMap<String, super::folder_create_journal::PendingFolderCreate>>>,
    pub(super) drive_account_key: Arc<str>,
    pub(super) pending_folder_dir: Option<Arc<PathBuf>>,
    /// Serializes uploads only when they target the same normalized path;
    /// unrelated file uploads remain parallel.
    upload_paths: Arc<UploadPathLocks>,
    pub(super) root: String,
    pub(super) api_base: Arc<str>,
    pub(super) persist_cache: bool,
    pub(super) request_timeout: Duration,
    /// Streaming downloads use per-socket inactivity deadlines rather than an
    /// overall request timeout, so an active large transfer has no wall-clock
    /// cap while a blackholed read still fails.
    pub(super) stream_agent: ureq::Agent,
}

impl GDriveBackend {
    /// Build from the stored refresh token (must already be connected via
    /// `cloud::authorize`). `root` is the forward-slash start folder.
    pub fn connect(root: &str) -> Result<Self, String> {
        let tokens = cloud::refresh_access(Provider::GDrive)?;
        let drive_account_key = load_drive_account_key(&tokens.access_token)?;
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
            pending_folder_creates: Arc::new(Mutex::new(HashMap::new())),
            drive_account_key: Arc::from(drive_account_key),
            pending_folder_dir: Some(Arc::new(super::folder_create_journal::record_dir())),
            upload_paths: Arc::new(UploadPathLocks {
                active: Mutex::new(HashSet::new()),
                ready: Condvar::new(),
            }),
            root: super::core::norm(root),
            api_base: Arc::from(super::api::API),
            persist_cache: true,
            request_timeout: super::api::DRIVE_REQUEST_TIMEOUT,
            stream_agent: stream_agent(super::api::DRIVE_REQUEST_TIMEOUT),
        })
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
        let mut ids = HashMap::new();
        ids.insert(String::new(), "root".to_string());
        let refresh_token = "test-refresh".to_string();
        let drive_account_key =
            super::folder_create_journal::account_key("test-drive-permission-id");
        Self {
            tokens: Arc::new(Mutex::new(cloud::Tokens {
                access_token: "test-token".into(),
                refresh_token,
                expires_at: i64::MAX,
            })),
            ids: Arc::new(Mutex::new(ids)),
            untrusted_ids: Arc::new(Mutex::new(HashSet::new())),
            mimes: Arc::new(Mutex::new(HashMap::new())),
            listed: Arc::new(Mutex::new(HashSet::new())),
            create_lock: Arc::new(Mutex::new(())),
            mutation_lock: Arc::new(Mutex::new(())),
            pending_upload_ids: Arc::new(Mutex::new(HashMap::new())),
            pending_folder_creates: Arc::new(Mutex::new(HashMap::new())),
            drive_account_key: Arc::from(drive_account_key),
            pending_folder_dir: pending_folder_dir.map(Arc::new),
            upload_paths: Arc::new(UploadPathLocks {
                active: Mutex::new(HashSet::new()),
                ready: Condvar::new(),
            }),
            root: String::new(),
            api_base: Arc::from(api_base),
            persist_cache: false,
            request_timeout,
            stream_agent: stream_agent(request_timeout),
        }
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

    pub(super) fn pending_folder_creates_guard(
        &self,
    ) -> io::Result<
        MutexGuard<'_, HashMap<String, super::folder_create_journal::PendingFolderCreate>>,
    > {
        self.pending_folder_creates
            .lock()
            .map_err(|_| io::Error::other("Drive-Ordnerreservierungs-Cache vergiftet"))
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

/// Drive publishes `user.permissionId` as the requesting user's opaque grantee
/// ID. Unlike a refresh token, it remains the same when OAuth credentials are
/// refreshed or re-authorized, so durable mutation records stay discoverable.
fn load_drive_account_key(access_token: &str) -> Result<String, String> {
    let url = format!("{}/about?fields=user(permissionId)", super::api::API);
    let bearer = format!("Bearer {access_token}");
    let response = ureq::get(&url)
        .timeout(super::api::DRIVE_REQUEST_TIMEOUT)
        .set("Authorization", &bearer)
        .call()
        .map_err(|error| format!("Drive account identity request failed: {error}"))?;
    let body = response
        .into_string()
        .map_err(|error| format!("Drive account identity response failed: {error}"))?;
    parse_drive_account_key(&body)
}

fn parse_drive_account_key(body: &str) -> Result<String, String> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("Drive account identity response is invalid: {error}"))?;
    let permission_id = json["user"]["permissionId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Drive account identity response has no permissionId".to_string())?;
    Ok(super::folder_create_journal::account_key(permission_id))
}

fn stream_agent(inactivity_timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(super::api::DRIVE_CONNECT_TIMEOUT)
        .timeout_read(inactivity_timeout)
        .timeout_write(inactivity_timeout)
        .build()
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
