use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct GDriveBackend {
    pub(super) tokens: Mutex<cloud::Tokens>,
    /// path (forward-slash, no trailing slash; "" == root) -> fileId
    pub(super) ids: Mutex<HashMap<String, String>>,
    /// path -> mimeType (so we know which files are Google-Docs editors that
    /// must be exported instead of downloaded).
    pub(super) mimes: Mutex<HashMap<String, String>>,
    /// Directories whose children are fully known (enumerated by `list_dir`, or
    /// freshly created and therefore empty). For such a parent, a path NOT in
    /// `ids` is known-absent -> we can create it directly and skip the per-file
    /// existence probe. This halves the round-trips during a large first sync.
    pub(super) listed: Mutex<HashSet<String>>,
    /// Serializes folder creation so concurrent transfers can't create the same
    /// directory twice (Drive happily makes duplicate same-name folders).
    pub(super) create_lock: Mutex<()>,
    pub(super) root: String,
}

impl GDriveBackend {
    /// Build from the stored refresh token (must already be connected via
    /// `cloud::authorize`). `root` is the forward-slash start folder.
    pub fn connect(root: &str) -> Result<Self, String> {
        let tokens = cloud::refresh_access(Provider::GDrive)?;
        let mut ids = HashMap::new();
        ids.insert(String::new(), "root".to_string());
        Ok(GDriveBackend {
            tokens: Mutex::new(tokens),
            ids: Mutex::new(ids),
            mimes: Mutex::new(HashMap::new()),
            listed: Mutex::new(HashSet::new()),
            create_lock: Mutex::new(()),
            root: super::core::norm(root),
        })
    }
}
