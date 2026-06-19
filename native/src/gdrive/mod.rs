//! Google Drive backend (#19, slice 2) - `impl vfs::Backend` over the Drive v3
//! REST API, so Drive plugs into the same browse/scan/sync machinery as SFTP &
//! co. Auth (PKCE OAuth, token refresh) lives in `cloud.rs`; this module only
//! makes authenticated REST calls.
//!
//! Drive is ID-addressed, not path-addressed, so we keep a `path -> fileId`
//! cache and resolve lazily from the My-Drive root (`"root"`). Forward-slash
//! paths are the app's convention; `"/"` is the Drive root.
//!
//! NOTE: this code follows the documented Drive v3 API but cannot be exercised
//! in the headless build env (no OAuth client). It compiles for host +
//! windows-gnu and is gated behind an explicit, user-configured connection.

mod api;
mod auth;
mod backend;
mod core;
mod metadata;
mod transfer;

use crate::cloud::{self, Provider};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct GDriveBackend {
    tokens: Mutex<cloud::Tokens>,
    /// path (forward-slash, no trailing slash; "" == root) -> fileId
    ids: Mutex<HashMap<String, String>>,
    /// path -> mimeType (so we know which files are Google-Docs editors that
    /// must be exported instead of downloaded).
    mimes: Mutex<HashMap<String, String>>,
    /// Directories whose children are fully known (enumerated by `list_dir`, or
    /// freshly created and therefore empty). For such a parent, a path NOT in
    /// `ids` is known-absent -> we can create it directly and skip the per-file
    /// existence probe. This halves the round-trips during a large first sync.
    listed: Mutex<HashSet<String>>,
    /// Serializes folder creation so concurrent transfers can't create the same
    /// directory twice (Drive happily makes duplicate same-name folders).
    create_lock: Mutex<()>,
    root: String,
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
            root: core::norm(root),
        })
    }
}
