//! The mount engines behind the Android documents provider (DRV-M8, strand
//! S2 C3/C4): one engine per saved connection, Google Drive, unlocked vault
//! and paired device, opened on first use in the app process. Each engine
//! keeps its journal below `<app data>/docs-cache/<mount id>` under the
//! cache lease, uses the Posix name rules and reads read-only handles by
//! ranges. A device is written to only where it lets this phone write; a
//! room device, a backend without safe staged writes and a vault without
//! them are read-only roots.
//!
//! The device folder `devices/<key>` is virtual: its engine runs over the
//! whole Share backend of the device, whose top level lists the device's
//! shared folders (the peer refuses writes there itself).
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use sha2::{Digest, Sha256};

use super::docs_ids::{DocId, RootId};
use crate::mobile::runtime::lock;
use crate::mount::os::linux_os::cache_lease::CacheLease;
use crate::mount::{
    device_key_of, open_target_of, BackendRoot, DeviceKind, DeviceRow, MountEngine,
    MountFilesystem, MountId, MountMode, MountRuntimeConfig, OpenDisposition, OpenFileOptions,
    PathDialect, PeerMountTarget, RoomRef,
};
use crate::share::{device_view, ShareProfiles, ShareStatus};
use crate::vfs::BackendHandle;

/// `EXTRA_ERROR` of an offline device (DocumentsUI shows it).
pub(super) const OFFLINE_TEXT: &str = "Gerät nicht erreichbar";
const CACHE_DIR: &str = "docs-cache";
const ROOT_FILE_SUFFIX: &str = ".root";

/// Which engine serves a document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum EngineKey {
    Root(RootId),
    /// One paired device by its key (`mount::device_key`).
    Device(String),
}

impl EngineKey {
    /// The engine of `doc`; `None` for the virtual level `devices`.
    pub(super) fn of(doc: &DocId) -> Option<(Self, String)> {
        match doc.device_part() {
            Some((key, path)) => Some((Self::Device(key.to_string()), path)),
            None if doc.root == RootId::Devices => None,
            None => Some((Self::Root(doc.root.clone()), doc.engine_path())),
        }
    }

    pub(super) fn root(&self) -> RootId {
        match self {
            Self::Root(root) => root.clone(),
            Self::Device(_) => RootId::Devices,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Root(root) => root.to_string(),
            Self::Device(key) => format!("devices/{key}"),
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label.strip_prefix("devices/") {
            Some(key) => DocId::parse(label).map(|_| Self::Device(key.to_string())),
            None => RootId::parse(label).map(Self::Root),
        }
    }

    /// Stable across restarts, so the journal of a crashed process is found.
    fn mount_id(&self) -> io::Result<MountId> {
        let digest = Sha256::digest(self.label().as_bytes());
        let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        MountId::parse(&format!("docs-{hex}"))
    }
}

/// An open engine.
pub(super) struct Slot {
    pub(super) fs: Arc<dyn MountFilesystem>,
    pub(super) writable: bool,
    _lease: CacheLease,
}

/// What a source hands to `open`: backend, root, writability, storage.
pub(super) struct Source {
    pub(super) backend: BackendHandle,
    pub(super) root: BackendRoot,
    pub(super) writable: bool,
    pub(super) storage: Option<crate::mount::MountStorage>,
}

#[derive(Default)]
struct Table {
    open: HashMap<EngineKey, Arc<Slot>>,
    /// One opener per key at a time (a second lease would fail).
    opening: HashMap<EngineKey, Arc<Mutex<()>>>,
}

fn table() -> &'static Mutex<Table> {
    static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(Table::default()))
}

pub(super) fn cache_root() -> io::Result<PathBuf> {
    crate::mount::prepare_spool_root(&crate::support_dirs::app_data_dir().join(CACHE_DIR))
}

/// The open engine of `key`, opening it first (blocks on the network).
pub(super) fn engine(key: &EngineKey) -> io::Result<Arc<Slot>> {
    if let Some(slot) = lock(table()).open.get(key) {
        return Ok(Arc::clone(slot));
    }
    let gate = Arc::clone(lock(table()).opening.entry(key.clone()).or_default());
    let _serial = lock(&gate);
    if let Some(slot) = lock(table()).open.get(key) {
        return Ok(Arc::clone(slot));
    }
    let slot = Arc::new(open(key)?);
    lock(table()).open.insert(key.clone(), Arc::clone(&slot));
    Ok(slot)
}

/// The open engine of `key` without opening one.
pub(super) fn opened(key: &EngineKey) -> Option<Arc<Slot>> {
    lock(table()).open.get(key).cloned()
}

/// Forgets every engine of `root`; open handles end separately
/// (`docs_io::revoke`). Unpublished changes stay in the journal.
pub(super) fn close_root(root: &RootId) -> Vec<Arc<Slot>> {
    let mut table = lock(table());
    let keys: Vec<EngineKey> = table
        .open
        .keys()
        .filter(|key| key.root() == *root)
        .cloned()
        .collect();
    keys.iter()
        .filter_map(|key| table.open.remove(key))
        .collect()
}

fn open(key: &EngineKey) -> io::Result<Slot> {
    // The lease first: it creates the mount folder the vault key lives in.
    let cache = cache_root()?;
    let id = key.mount_id()?;
    let lease = CacheLease::acquire(&cache, &id)?;
    let source = match key {
        EngineKey::Root(RootId::Conn(account)) => conn_source(account)?,
        EngineKey::Root(RootId::GDrive) => {
            let (backend, _) = crate::connect::open_gdrive("").map_err(io::Error::other)?;
            source(backend, BackendRoot::parse("/")?, true)
        }
        EngineKey::Root(RootId::Vault(site)) => {
            super::docs_vault::source(site, &cache.join(id.as_str()))?
        }
        EngineKey::Device(device) => device_source(device)?,
        EngineKey::Root(RootId::Devices) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the device tree has no engine of its own",
            ))
        }
    };
    let mode = if source.writable {
        MountMode::ReadWrite
    } else {
        MountMode::ReadOnly
    };
    let config = MountRuntimeConfig::new(id.clone(), mode)
        .with_path_dialect(PathDialect::Posix)
        .with_range_reads_on_read_only_handles();
    let rooted = source.root.as_str() != "/";
    let engine = match source.storage {
        Some(storage) => {
            MountEngine::open_host_cache_with(config, source.backend, &cache, storage)?
        }
        None if rooted => MountEngine::open_rooted(config, source.root, source.backend, &cache)?,
        None => MountEngine::open_host_cache(config, source.backend, &cache)?,
    };
    let space = crate::mount::os::linux_os::cache_space::CacheDiskSpace::new(&cache)?;
    let engine = engine.with_cache_space_probe(Arc::new(space));
    if !rooted {
        engine.prepare_host_remote()?;
    }
    // Changes a crashed process left behind go out now; a failure keeps
    // them in the journal for `docs.retryPending`.
    let _ = engine.retry_pending_changes();
    let marker = cache.join(format!("{}{ROOT_FILE_SUFFIX}", id.as_str()));
    crate::support_dirs::write_private_atomic(&marker, key.label().as_bytes())?;
    Ok(Slot {
        fs: Arc::new(engine),
        writable: source.writable,
        _lease: lease,
    })
}

/// A writable source only where the backend has safe staged writes.
pub(super) fn source(backend: BackendHandle, root: BackendRoot, allowed: bool) -> Source {
    let staged = backend
        .staged_write_capabilities(root.as_str())
        .supports_mounted_writes();
    Source {
        backend,
        root,
        writable: allowed && staged,
        storage: None,
    }
}

fn conn_source(account: &str) -> io::Result<Source> {
    let connection = saved_connection(account)?;
    let root = if connection.root.trim().is_empty() {
        "/".to_string()
    } else {
        connection.root.clone()
    };
    let (backend, _) = crate::connect::open_saved_at_for_mount(
        &connection,
        &root,
        crate::mount::MountRootSecurity::Enforced,
    )
    .map_err(io::Error::other)?;
    Ok(source(backend, BackendRoot::parse(&root)?, true))
}

pub(super) fn saved_connection(account: &str) -> io::Result<crate::creds::SavedConnection> {
    crate::creds::load_connections_checked()
        .map_err(io::Error::other)?
        .into_iter()
        .find(|connection| connection.protocol.is_url() && connection.account() == account)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Verbindung nicht gefunden"))
}

fn device_source(key: &str) -> io::Result<Source> {
    let profiles = profiles();
    let row = device_rows(&profiles)
        .into_iter()
        .find(|row| row.key == key)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Gerät nicht gefunden"))?;
    if !row.online {
        return Err(io::Error::new(io::ErrorKind::HostUnreachable, OFFLINE_TEXT));
    }
    let allowed = match &row.target {
        // Rooms are read-only (strand S2 C4); a contact is written to unless
        // this phone's grant says otherwise, as on the desktop tree.
        PeerMountTarget::RoomDevice { .. } => false,
        PeerMountTarget::Direct { contact_id } => profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.id == *contact_id)
            .is_none_or(|contact| device_view::granted_write(&profiles, contact) != Some(false)),
    };
    let (_, backend, _) = crate::daemon::open_share_backend(open_target_of(&row.target))
        .map_err(|_| io::Error::new(io::ErrorKind::HostUnreachable, OFFLINE_TEXT))?;
    Ok(source(backend, BackendRoot::parse("/")?, allowed))
}

/// The Share profiles of the last poll, else from disk.
pub(super) fn profiles() -> ShareProfiles {
    super::share_state::with_state(|state| state.profiles.clone()).unwrap_or_else(|| {
        ShareProfiles::load_checked(super::share_state::default_home()).unwrap_or_default()
    })
}

/// One row per paired device, as the desktop tree lists them (DRV-M6).
pub(super) fn device_rows(profiles: &ShareProfiles) -> Vec<DeviceRow> {
    let now = crate::mobile::runtime::now_ms() / 1000;
    let own = super::share_state::with_state(|state| state.identity.clone())
        .map(|identity| identity.device_id);
    let mut rows: Vec<DeviceRow> = profiles
        .direct_contacts
        .iter()
        .filter(|contact| contact.status != ShareStatus::IdentityConflict)
        .map(|contact| {
            let target = PeerMountTarget::Direct {
                contact_id: contact.id.clone(),
            };
            DeviceRow {
                key: device_key_of(&target),
                name: contact.display_name.clone(),
                kind: DeviceKind::Direct,
                room: None,
                online: device_view::online(contact, now),
                target,
            }
        })
        .collect();
    for room in &profiles.rooms {
        for member in &room.members {
            if own.as_deref() == Some(member.device_id.as_str()) {
                continue;
            }
            let target = PeerMountTarget::RoomDevice {
                room_id: room.id.clone(),
                device_id: member.device_id.clone(),
            };
            rows.push(DeviceRow {
                key: device_key_of(&target),
                name: member.device_name.clone(),
                kind: DeviceKind::Room,
                room: Some(RoomRef {
                    id: room.id.clone(),
                    name: room.name.clone(),
                }),
                online: device_view::reachable(&member.status),
                target,
            });
        }
    }
    rows.sort_by(|left, right| left.key.cmp(&right.key));
    rows
}

/// Creates an empty file (`dir == false`) or a folder at `path`.
pub(super) fn create(slot: &Slot, path: &str, dir: bool) -> io::Result<()> {
    if !slot.writable {
        return Err(read_only());
    }
    if dir {
        return slot.fs.mkdir(path).map(|_| ());
    }
    let options = OpenFileOptions {
        writable: true,
        disposition: OpenDisposition::CreateNew,
    };
    let handle = slot.fs.open_file(path, options)?;
    let flushed = slot.fs.flush(handle);
    let closed = slot.fs.close(handle);
    flushed.and(closed)
}

/// Deletes `path`; a folder with its content, deepest first. Links and
/// special files are never listed by the engine, so a folder that still
/// holds one stays (with `DirectoryNotEmpty`) instead of being followed.
pub(super) fn delete(slot: &Slot, path: &str) -> io::Result<()> {
    if !slot.writable {
        return Err(read_only());
    }
    let meta = slot.fs.stat(path)?;
    if meta.is_dir && !meta.is_symlink {
        for child in slot.fs.list_dir(path)? {
            if child.is_symlink || child.special {
                continue;
            }
            delete(
                slot,
                &format!("{}/{}", path.trim_end_matches('/'), child.name),
            )?;
        }
    }
    slot.fs.delete(path, meta.is_dir && !meta.is_symlink)
}

pub(super) fn rename(slot: &Slot, from: &str, to: &str) -> io::Result<()> {
    if !slot.writable {
        return Err(read_only());
    }
    slot.fs.rename(from, to, false).map(|_| ())
}

fn read_only() -> io::Error {
    io::Error::new(
        io::ErrorKind::ReadOnlyFilesystem,
        "Dieser Ordner ist schreibgeschützt",
    )
}

/// Unpublished changes of every engine, opened or not (`docs.pending`).
pub(super) fn pending() -> usize {
    let open: Vec<(EngineKey, Arc<Slot>)> = lock(table())
        .open
        .iter()
        .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
        .collect();
    let mut total = 0;
    for (_, slot) in &open {
        total += slot.fs.dirty_entries().map_or(0, |entries| entries.len());
    }
    for (key, id) in recorded() {
        if open.iter().any(|(known, _)| *known == key) {
            continue;
        }
        if let Ok(cache) = cache_root() {
            total += crate::mount::os::linux_os::audit_pending(&cache, &id).unwrap_or(0);
        }
    }
    total
}

/// Publishes what waits (`docs.retryPending`): open engines, and engines
/// with a journal from an earlier process. A locked vault waits for its
/// unlock; nothing of it is opened. Returns the keys that still failed.
pub(super) fn retry_pending(released: &dyn Fn(&RootId) -> bool) -> Vec<String> {
    let mut failed = Vec::new();
    let cache = cache_root().ok();
    for (key, id) in recorded() {
        let waiting = cache.as_ref().map_or(0, |cache| {
            crate::mount::os::linux_os::audit_pending(cache, &id).unwrap_or(0)
        });
        let open = opened(&key);
        if open.is_none() && (waiting == 0 || !released(&key.root())) {
            continue;
        }
        let result = match open {
            Some(slot) => slot.fs.retry_pending_changes(),
            None => engine(&key).and_then(|slot| slot.fs.retry_pending_changes()),
        };
        if result.is_err() {
            failed.push(key.label());
        }
    }
    failed
}

/// Engines this installation ever opened, by their marker files.
fn recorded() -> Vec<(EngineKey, MountId)> {
    let Ok(cache) = cache_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&cache) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let id = MountId::parse(name.strip_suffix(ROOT_FILE_SUFFIX)?).ok()?;
            let label = crate::support_dirs::read_private_text(&entry.path(), 4096).ok()?;
            let key = EngineKey::from_label(label.trim())?;
            (key.mount_id().ok()? == id).then_some((key, id))
        })
        .collect()
}
