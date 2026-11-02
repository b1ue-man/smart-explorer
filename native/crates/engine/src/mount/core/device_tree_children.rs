//! The child engines of the device tree (DRV-M6, U17-15 to U17-18): one
//! `MountEngine` per shared folder of a device, opened at the first access
//! over a `LazyChildBackend` (`device_tree_backend.rs`). `children.json`
//! remembers which children exist, so the journal of a child that was never
//! published is found again after a restart, with no network.
use std::any::Any;
use std::fs;
use std::io::{self, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cache_policy::MountCachePolicy;
use super::cache_space::CacheSpaceProbe;
use super::device_tree_backend::LazyChildBackend;
use super::device_tree_names::DEVICES_MOUNT_ID;
use super::engine::MountEngine;
use super::metadata_policy::MountMetadataPolicy;
use super::mount_source::PeerMountTarget;
use super::path_dialect::PathDialect;
use super::spool_storage::MountStorage;
use super::types::{MountId, MountMode, MountRuntimeConfig};
use crate::vfs::BackendHandle;

const BOOK_VERSION: u32 = 1;
const BOOK_FILE: &str = "children.json";

/// The identity of one child: the device and the name of its shared folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ChildKey {
    pub key: String,
    pub name: String,
}

/// The cache id of a child: 32 hex digits of the SHA-256 of the tree, the
/// device and the root (decision C5).
pub(super) fn kind_id(key: &str, root: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"se-devices-child\0");
    hasher.update(DEVICES_MOUNT_ID.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.as_bytes());
    hasher.update([0u8]);
    hasher.update(root.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What opening a child needs from the tree.
pub(super) struct ChildFactory {
    pub(super) spool_root: PathBuf,
    pub(super) dialect: PathDialect,
    pub(super) metadata: MountMetadataPolicy,
    pub(super) cache: MountCachePolicy,
    pub(super) space: Arc<dyn CacheSpaceProbe>,
}

/// An open child: its engine, its backend and its cache lease.
pub(super) struct ChildEngine {
    // Field order is drop order: the engine ends before its cache is
    // released.
    pub(super) engine: Arc<MountEngine>,
    pub(super) backend: Arc<LazyChildBackend>,
    pub(super) mode: MountMode,
    /// Holds (`Pin`): open files, delete transactions and running calls.
    pub(super) handles: AtomicUsize,
    retrying: AtomicBool,
    prepared: AtomicBool,
    _lease: Mutex<Option<Box<dyn Any + Send>>>,
}

/// A hold on an open engine: while it lives the engine is not closed. It is
/// taken under the lock of the tree's table of engines, so the sweep that
/// closes idle engines cannot slip in between.
pub(super) struct Pin(Arc<ChildEngine>);

impl Pin {
    pub(super) fn new(child: &Arc<ChildEngine>) -> Self {
        child.handles.fetch_add(1, Ordering::AcqRel);
        Self(Arc::clone(child))
    }
}

impl Clone for Pin {
    fn clone(&self) -> Self {
        Self::new(&self.0)
    }
}

impl Deref for Pin {
    type Target = ChildEngine;

    fn deref(&self) -> &ChildEngine {
        &self.0
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.0.handles.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ChildFactory {
    pub(super) fn open(
        &self,
        id: &str,
        mode: MountMode,
        backend: Arc<LazyChildBackend>,
        lease: Box<dyn Any + Send>,
    ) -> io::Result<Arc<ChildEngine>> {
        let id = MountId::parse(id)?;
        let runtime = MountRuntimeConfig::new(id, mode)
            .with_metadata_policy(self.metadata)
            .with_cache_policy(self.cache)
            .with_path_dialect(self.dialect);
        let handle: BackendHandle = backend.clone();
        let engine = MountEngine::open_host_cache_with(
            runtime,
            handle,
            &self.spool_root,
            MountStorage::plain(),
        )?
        .with_cache_space_probe(Arc::clone(&self.space));
        Ok(Arc::new(ChildEngine {
            engine: Arc::new(engine),
            backend,
            mode,
            handles: AtomicUsize::new(0),
            retrying: AtomicBool::new(false),
            prepared: AtomicBool::new(false),
            _lease: Mutex::new(Some(lease)),
        }))
    }
}

impl ChildEngine {
    /// Unpublished changes, deletes and intents; an unreadable journal
    /// counts as one.
    pub(super) fn pending(&self) -> usize {
        self.engine
            .dirty_entries()
            .map_or(1, |entries| entries.len())
    }

    /// No open handle and no journal entry: the engine may be closed.
    pub(super) fn idle(&self) -> bool {
        self.handles.load(Ordering::Acquire) == 0 && self.pending() == 0
    }

    /// Publishes the journal; the remote root is checked once before the
    /// first try. Only one try runs at a time.
    pub(super) fn retry(&self) -> io::Result<()> {
        if self.retrying.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let result = (|| {
            if !self.prepared.load(Ordering::Acquire) {
                self.engine.prepare_host_remote()?;
                self.prepared.store(true, Ordering::Release);
            }
            self.engine.retry_pending_changes()
        })();
        self.retrying.store(false, Ordering::Release);
        result
    }
}

/// One line of `children.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Record {
    pub id: String,
    pub key: String,
    pub target: PeerMountTarget,
    pub root: String,
    /// The segment of the shared folder below the device folder.
    pub name: String,
    pub mode: RecordMode,
    /// Journal entries at the last look (status display only).
    #[serde(default)]
    pub pending: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RecordMode {
    ReadOnly,
    ReadWrite,
}

impl From<MountMode> for RecordMode {
    fn from(mode: MountMode) -> Self {
        match mode {
            MountMode::ReadOnly => Self::ReadOnly,
            MountMode::ReadWrite => Self::ReadWrite,
        }
    }
}

impl From<RecordMode> for MountMode {
    fn from(mode: RecordMode) -> Self {
        match mode {
            RecordMode::ReadOnly => Self::ReadOnly,
            RecordMode::ReadWrite => Self::ReadWrite,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Book {
    version: u32,
    children: Vec<Record>,
}

fn book_path(spool_root: &Path) -> PathBuf {
    spool_root.join(DEVICES_MOUNT_ID).join(BOOK_FILE)
}

/// The children of an earlier run; no file means none. A file this version
/// cannot read is an error: its journals must not be forgotten.
pub(super) fn load_book(spool_root: &Path) -> io::Result<Vec<Record>> {
    let body = match fs::read(book_path(spool_root)) {
        Ok(body) => body,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let book: Book = serde_json::from_slice(&body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if book.version != BOOK_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "children.json hat eine unbekannte Version",
        ));
    }
    Ok(book.children)
}

/// Replaces `children.json` through a temporary file in the same folder.
pub(super) fn store_book(spool_root: &Path, records: &[Record]) -> io::Result<()> {
    let path = book_path(spool_root);
    let body = serde_json::to_vec_pretty(&Book {
        version: BOOK_VERSION,
        children: records.to_vec(),
    })
    .map_err(io::Error::other)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// How many devices have changes waiting for a connection, from the last
/// `children.json` of the host; 0 when it is unknown.
pub fn pending_devices(spool_root: &Path) -> u32 {
    let Ok(records) = load_book(spool_root) else {
        return 0;
    };
    let mut keys: Vec<&str> = records
        .iter()
        .filter(|record| record.pending > 0)
        .map(|record| record.key.as_str())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    u32::try_from(keys.len()).unwrap_or(u32::MAX)
}
