//! The mask store: `<app data>/masks/lists.sealed` (sealed document),
//! `state.json` (no secret, for the agent hook) and `store.lock`. Writers
//! hold a process mutex and the file lock across the whole read, change and
//! write, so concurrent processes (several `se mcp`, GUI, CLI) never hand
//! out one placeholder twice. Reading fails closed: a store that exists but
//! cannot be read (missing key, other key, damaged, older copy) is an error,
//! and `lockdown_state` answers `Unknown`.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::side::{self, Side};
use crate::masks::core::doc::{
    self, decode_key, encode_key, generate_key, StateDoc, StoreDoc, StoreKey, KEY_BYTES,
    MAX_SEALED_BYTES, MAX_STATE_BYTES,
};
use crate::masks::core::mask_set::{Allocate, MaskSet};
use crate::masks::core::model::{LockdownState, MaskError};
use crate::masks::core::tables::{allocate, Need, Tables};

pub(crate) const SEALED_FILE: &str = "lists.sealed";
pub(crate) const STATE_FILE: &str = "state.json";
const LOCK_FILE: &str = "store.lock";

static PROCESS_LOCK: Mutex<()> = Mutex::new(());

/// Handle of a mask store; clones share their cache.
#[derive(Clone)]
pub struct MaskStore {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) dir: PathBuf,
    key: KeySource,
    pub(crate) side: Side,
    cache: Mutex<Option<Arc<MaskSet>>>,
}

enum KeySource {
    Secrets,
    Fixed(StoreKey),
}

pub(crate) struct WriteGuard {
    _process: MutexGuard<'static, ()>,
    _file: std::fs::File,
}

impl std::fmt::Debug for MaskStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaskStore")
            .field("dir", &self.inner.dir)
            .finish_non_exhaustive()
    }
}

/// The lockdown state of the user's store, without its key (agent hook).
pub fn lockdown_state() -> LockdownState {
    lockdown_state_in(&default_dir(), &Side::Secrets)
}

fn default_dir() -> PathBuf {
    crate::support_dirs::app_data_dir().join("masks")
}

impl MaskStore {
    /// The store of the user (`<app data>/masks`, key in the secret store).
    pub fn open_default() -> Result<Self, MaskError> {
        Ok(Self::new(default_dir(), KeySource::Secrets, Side::Secrets))
    }

    /// A store in `dir` with a fixed key; its side records live in memory
    /// (tests, fixtures).
    pub fn open_at(dir: impl Into<PathBuf>, key: [u8; KEY_BYTES]) -> Result<Self, MaskError> {
        let dir = dir.into();
        let side = Side::Memory(dir.clone());
        Ok(Self::new(dir, KeySource::Fixed(StoreKey::new(key)), side))
    }

    fn new(dir: PathBuf, key: KeySource, side: Side) -> Self {
        Self {
            inner: Arc::new(Inner {
                dir,
                key,
                side,
                cache: Mutex::new(None),
            }),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// The masking set of the enabled lists, rebuilt when the lists changed
    /// (also in another process). Errors mean: fail closed.
    pub fn active(&self) -> Result<Arc<MaskSet>, MaskError> {
        let state = self.read_state().ok().flatten();
        {
            let cache = self.cache();
            if let (Some(set), Some(state)) = (cache.as_ref(), state) {
                if set.config_generation() == state.config_generation {
                    return Ok(Arc::clone(set));
                }
            }
        }
        let set = match self.load()? {
            None => MaskSet::empty(),
            Some(doc) => {
                if state != Some(doc.state()) {
                    // Masking works without it; the hook keeps the older
                    // state until the next successful write.
                    let _ = self.repair_state();
                }
                let allocator: Arc<dyn Allocate> = Arc::new(StoreAllocator {
                    store: Arc::downgrade(&self.inner),
                });
                MaskSet::build(&doc, Some(allocator))?
            }
        };
        let set = Arc::new(set);
        *self.cache() = Some(Arc::clone(&set));
        Ok(set)
    }

    /// The lockdown state of this store (`lockdown_state` for its directory).
    pub fn lockdown_state(&self) -> LockdownState {
        lockdown_state_in(&self.inner.dir, &self.inner.side)
    }

    fn cache(&self) -> MutexGuard<'_, Option<Arc<MaskSet>>> {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn invalidate(&self) {
        *self.cache() = None;
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.inner.dir.join(name)
    }

    /// Process mutex and file lock for one read-change-write.
    pub(crate) fn write_guard(&self) -> Result<WriteGuard, MaskError> {
        let process = PROCESS_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        crate::support_dirs::ensure_private_dir(&self.inner.dir).map_err(io)?;
        let file = crate::support_dirs::open_private_lock(&self.path(LOCK_FILE)).map_err(io)?;
        file.lock().map_err(io)?;
        Ok(WriteGuard {
            _process: process,
            _file: file,
        })
    }

    /// The document, `None` when no store exists. Fails closed.
    pub(crate) fn load(&self) -> Result<Option<StoreDoc>, MaskError> {
        let sealed = read_bounded(&self.path(SEALED_FILE), MAX_SEALED_BYTES)
            .map_err(|e| MaskError::Unreadable(format!("{SEALED_FILE}: {e}")))?;
        let Some(sealed) = sealed else {
            let state = read_bounded(&self.path(STATE_FILE), MAX_STATE_BYTES)
                .map_err(|e| MaskError::Unreadable(format!("{STATE_FILE}: {e}")))?;
            if state.is_some() {
                return Err(MaskError::Unreadable(format!("{SEALED_FILE} fehlt")));
            }
            return match self.inner.side.get(side::MARKER) {
                Ok(None) => Ok(None),
                Ok(Some(_)) => Err(MaskError::Unreadable(
                    "der Maskenspeicher wurde gelöscht".into(),
                )),
                Err(error) => Err(MaskError::Unreadable(error)),
            };
        };
        let key = self
            .key(false)?
            .ok_or_else(|| MaskError::Unreadable("Schlüssel fehlt im Geheimnisspeicher".into()))?;
        let doc = doc::open(&sealed, &key).map_err(MaskError::Unreadable)?;
        let recorded = self
            .inner
            .side
            .generation()
            .map_err(MaskError::Unreadable)?;
        if doc.config_generation < recorded {
            return Err(MaskError::Unreadable(format!(
                "älterer Stand {} statt {recorded}",
                doc.config_generation
            )));
        }
        Ok(Some(doc))
    }

    /// Saves `doc` as the successor of `before`. The caller holds the write
    /// guard. A switch-on of the lockdown reaches the hook's records before
    /// the sealed file, a switch-off after it.
    pub(crate) fn save(&self, doc: &mut StoreDoc, before: &StoreDoc) -> Result<(), MaskError> {
        doc.generation = before.generation.max(doc.generation) + 1;
        let key = self
            .key(true)?
            .ok_or_else(|| MaskError::Io("Schlüssel nicht verfügbar".into()))?;
        self.inner
            .side
            .set(side::MARKER, "1")
            .map_err(MaskError::Io)?;
        let sealed = doc::seal(doc, &key).map_err(MaskError::Io)?;
        let state = doc.state().encode();
        let lockdown = doc.lockdown();
        if lockdown {
            self.inner
                .side
                .set(side::LOCKDOWN, "1")
                .map_err(MaskError::Io)?;
            self.write(STATE_FILE, &state)?;
        }
        self.write(SEALED_FILE, &sealed)?;
        self.write(STATE_FILE, &state)?;
        if !lockdown {
            self.inner
                .side
                .set(side::LOCKDOWN, "0")
                .map_err(MaskError::Io)?;
        }
        if doc.config_generation != before.config_generation {
            // The file is saved; a failed record only weakens the rollback
            // check until the next change, it never blocks the store.
            let _ = self
                .inner
                .side
                .set(side::GENERATION, &doc.config_generation.to_string());
        }
        Ok(())
    }

    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> Result<(), MaskError> {
        crate::support_dirs::write_private_atomic(&self.path(name), bytes)
            .map_err(|e| MaskError::Io(format!("{name} schreiben: {e}")))
    }

    pub(crate) fn read_state(&self) -> Result<Option<StateDoc>, String> {
        read_bounded(&self.path(STATE_FILE), MAX_STATE_BYTES)
            .map_err(|e| e.to_string())?
            .map(|bytes| StateDoc::decode(&bytes))
            .transpose()
    }

    /// Rewrites `state.json` and the side lockdown flag from the document.
    pub(crate) fn repair_state(&self) -> Result<(), MaskError> {
        let _guard = self.write_guard()?;
        if let Some(doc) = self.load()? {
            self.write_side_and_state(&doc)?;
        }
        Ok(())
    }

    pub(crate) fn write_side_and_state(&self, doc: &StoreDoc) -> Result<(), MaskError> {
        let flag = if doc.lockdown() { "1" } else { "0" };
        self.inner
            .side
            .set(side::LOCKDOWN, flag)
            .map_err(MaskError::Io)?;
        self.inner
            .side
            .set(side::MARKER, "1")
            .map_err(MaskError::Io)?;
        self.write(STATE_FILE, &doc.state().encode())
    }

    fn key(&self, create: bool) -> Result<Option<StoreKey>, MaskError> {
        match &self.inner.key {
            KeySource::Fixed(key) => Ok(Some(key.clone())),
            KeySource::Secrets => {
                match self
                    .inner
                    .side
                    .get(side::KEY)
                    .map_err(MaskError::Unreadable)?
                {
                    Some(text) => decode_key(&text).map(Some).map_err(MaskError::Unreadable),
                    None if create => {
                        let key = generate_key().map_err(MaskError::Io)?;
                        self.inner
                            .side
                            .set(side::KEY, &encode_key(&key))
                            .map_err(MaskError::Io)?;
                        Ok(Some(key))
                    }
                    None => Ok(None),
                }
            }
        }
    }

    /// Records assignments for `needs` under the write guard.
    fn allocate(&self, needs: &[Need]) -> Result<Tables, MaskError> {
        let _guard = self.write_guard()?;
        let mut doc = self
            .load()?
            .ok_or_else(|| MaskError::Unreadable("der Maskenspeicher fehlt".into()))?;
        let before = doc.clone();
        if allocate(&mut doc, needs) {
            self.save(&mut doc, &before)?;
        }
        Ok(Tables::from_doc(&doc))
    }
}

/// Saves new assignments of a masking set through its store.
struct StoreAllocator {
    store: Weak<Inner>,
}

impl StoreAllocator {
    fn store(&self) -> Result<MaskStore, String> {
        self.store
            .upgrade()
            .map(|inner| MaskStore { inner })
            .ok_or_else(|| "der Maskenspeicher ist geschlossen".to_string())
    }
}

impl Allocate for StoreAllocator {
    fn allocate(&self, needs: &[Need]) -> Result<Tables, String> {
        self.store()?.allocate(needs).map_err(|e| e.to_string())
    }

    fn reload(&self) -> Result<Tables, String> {
        let store = self.store()?;
        let doc = store.load().map_err(|e| e.to_string())?;
        Ok(doc.map(|doc| Tables::from_doc(&doc)).unwrap_or_default())
    }
}

/// Lockdown without the key: `state.json` or the side flag says on → `On`;
/// a store that exists or existed but whose state cannot be read →
/// `Unknown`; no store at all → `Off`.
pub(crate) fn lockdown_state_in(dir: &Path, side: &Side) -> LockdownState {
    match read_bounded(&dir.join(STATE_FILE), MAX_STATE_BYTES) {
        Ok(Some(bytes)) => match StateDoc::decode(&bytes) {
            Ok(state) if state.lockdown => LockdownState::On,
            Ok(_) => match side.get(side::LOCKDOWN) {
                Ok(Some(flag)) if flag.trim() == "1" => LockdownState::On,
                Ok(_) => LockdownState::Off,
                Err(_) => LockdownState::Unknown,
            },
            Err(_) => LockdownState::Unknown,
        },
        Ok(None) => match std::fs::symlink_metadata(dir.join(SEALED_FILE)) {
            Ok(_) => LockdownState::Unknown,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match side.get(side::MARKER) {
                    Ok(None) => LockdownState::Off,
                    _ => LockdownState::Unknown,
                }
            }
            Err(_) => LockdownState::Unknown,
        },
        Err(_) => LockdownState::Unknown,
    }
}

/// A private file read up to `max` bytes; `None` when it does not exist.
pub(crate) fn read_bounded(path: &Path, max: u64) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let file = crate::support_dirs::open_private_file(path)?;
    if file.metadata()?.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Datei größer als erlaubt",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Datei größer als erlaubt",
        ));
    }
    Ok(Some(bytes))
}

fn io(error: std::io::Error) -> MaskError {
    MaskError::Io(error.to_string())
}
