use super::journal::{Journal, PersistedDelete, PersistedEntry, RecoveredJournal};
use super::spool_storage::{MountStorage, SpoolHandle, SpoolStorage};
use super::stage_ledger::StageLedger;
use super::types::{MountId, NamespaceIntent};
use crate::local_access::{DirectoryHandle, EntryKind};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const ALLOCATION_ATTEMPTS: usize = 128;

#[path = "spool_audit.rs"]
mod audit;
#[path = "spool_recovery.rs"]
mod recovery;

pub(super) struct AllocatedSpool {
    pub name: String,
    pub file: SpoolHandle,
}

pub(super) struct WholeFileSpool {
    files: PathBuf,
    /// Content storage of every spool file (contract V1); path checks stay here.
    storage: Arc<dyn SpoolStorage>,
    journal: Mutex<Journal>,
    referenced: Mutex<HashSet<String>>,
    missing_recovery: HashSet<String>,
    cleanup_ready: bool,
    stages: StageLedger,
}

pub fn prepare_spool_root(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount spool root must be absolute",
        ));
    }
    reject_link_ancestors(path)?;
    ensure_directory(path)?;
    reject_link_ancestors(path)?;
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "canonical mount spool root is not absolute",
        ));
    }
    ensure_directory(&canonical)?;
    Ok(canonical)
}

fn reject_link_ancestors(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "mount spool path crosses a symbolic link: {}",
                        ancestor.display()
                    ),
                ));
            }
            Ok(metadata) if ancestor != path && !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "mount spool ancestor is not a directory: {}",
                        ancestor.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

impl WholeFileSpool {
    /// The plain storage of every drive without a vault.
    pub fn open(base: &Path, mount_id: &MountId) -> io::Result<(Self, RecoveredJournal)> {
        Self::open_with(base, mount_id, MountStorage::plain())
    }

    pub fn open_with(
        base: &Path,
        mount_id: &MountId,
        storage: MountStorage,
    ) -> io::Result<(Self, RecoveredJournal)> {
        ensure_directory(base)?;
        let root = base.join(mount_id.as_str());
        ensure_directory(&root)?;
        let files = root.join("files");
        ensure_directory(&files)?;
        recovery::require_journal_for_existing_files(&root, &files)?;
        let (journal, mut recovered) =
            Journal::open(&root.join("journal.jsonl"), Arc::clone(&storage.paths))?;
        let mut spool = Self {
            files,
            storage: storage.spool,
            journal: Mutex::new(journal),
            referenced: Mutex::new(
                recovered
                    .entries
                    .values()
                    .map(|entry| entry.spool_name.clone())
                    .collect(),
            ),
            missing_recovery: HashSet::new(),
            cleanup_ready: false,
            stages: StageLedger::open(&root, storage.paths),
        };
        let mut names = HashSet::new();
        for entry in recovered.entries.values_mut() {
            if !names.insert(entry.spool_name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "two recovery entries reference the same mount spool",
                ));
            }
            if recovery::validate_entry(&spool, entry)? {
                spool.missing_recovery.insert(entry.spool_name.clone());
            }
        }
        let referenced = recovered
            .entries
            .values()
            .map(|entry| entry.spool_name.clone())
            .collect::<HashSet<_>>();
        // A failed audit must never authorize cleanup from Drop.
        spool.cleanup_ready = true;
        spool.remove_orphan_clean_files(&referenced)?;
        Ok((spool, recovered))
    }

    pub(super) fn stages(&self) -> &StageLedger {
        &self.stages
    }

    /// Reads may start while a download still fills the spool (G4 ML4).
    pub(super) fn streams(&self) -> bool {
        self.storage.reads_while_writing()
    }

    pub fn allocate(&self) -> io::Result<AllocatedSpool> {
        for _ in 0..ALLOCATION_ATTEMPTS {
            let name = format!("{}.spool", random_hex()?);
            let path = self.files.join(&name);
            match self.storage.create_new(&path) {
                Ok(file) => {
                    return Ok(AllocatedSpool {
                        name,
                        file: SpoolHandle::new(file),
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique mount spool file",
        ))
    }

    pub fn open_file(&self, name: &str, writable: bool) -> io::Result<SpoolHandle> {
        validate_spool_name(name)?;
        let path = self.files.join(name);
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| recovery::file_error(&path, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount spool entry is not a plain file",
            ));
        }
        self.storage
            .open(&path, writable)
            .map(SpoolHandle::new)
            .map_err(|error| recovery::file_error(&path, error))
    }

    pub fn remove_file(&self, name: &str) -> io::Result<()> {
        validate_spool_name(name)?;
        let path = self.files.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "refusing to remove a non-file mount spool entry",
                ))
            }
            Ok(_) => match fs::remove_file(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub fn persist_entry(&self, entry: &PersistedEntry) -> io::Result<()> {
        if let Ok(mut referenced) = self.referenced.lock() {
            referenced.insert(entry.spool_name.clone());
        }
        lock(&self.journal)?.upsert_entry(entry)
    }

    pub fn is_recovery_referenced(&self, name: &str) -> io::Result<bool> {
        Ok(lock(&self.referenced)?.contains(name))
    }

    pub fn is_missing_recovery(&self, name: &str) -> bool {
        self.missing_recovery.contains(name)
    }

    pub fn forget_entry(&self, remote_path: &str, spool_name: &str) -> io::Result<()> {
        lock(&self.journal)?.forget_entry(remote_path)?;
        if let Ok(mut referenced) = self.referenced.lock() {
            referenced.remove(spool_name);
        }
        Ok(())
    }

    pub fn move_entry(&self, old_path: &str, entry: &PersistedEntry) -> io::Result<()> {
        lock(&self.journal)?.move_entry(old_path, entry)
    }

    pub fn persist_delete(&self, delete: &PersistedDelete) -> io::Result<()> {
        lock(&self.journal)?.upsert_delete(delete)
    }

    pub fn forget_delete(&self, token: u64) -> io::Result<()> {
        lock(&self.journal)?.forget_delete(token)
    }

    pub fn persist_namespace_conflict(&self, intent: &NamespaceIntent) -> io::Result<()> {
        lock(&self.journal)?.upsert_namespace_conflict(intent)
    }

    pub fn forget_namespace_conflict(&self, path: &str) -> io::Result<()> {
        lock(&self.journal)?.forget_namespace_conflict(path)
    }

    fn remove_orphan_clean_files(&self, referenced: &HashSet<String>) -> io::Result<()> {
        let mut first_error = None;
        for child in fs::read_dir(&self.files)? {
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let name = child.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_spool_name(name) || referenced.contains(name) {
                continue;
            }
            if let Err(error) = self.remove_file(name) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

pub(super) fn audit_recovery(
    base: &Path,
    mount_id: &MountId,
) -> io::Result<super::recovery_state::MountRecovery> {
    Ok(match audit_pending_state(base, mount_id)? {
        Some(0) => super::recovery_state::MountRecovery::Clean,
        Some(_) => super::recovery_state::MountRecovery::Required,
        None => super::recovery_state::MountRecovery::Unknown,
    })
}

/// Compatibility for callers that need an exact count. An unknown count is
/// an error, never a clean cache. OS callers hold the exclusive existing lease.
pub(super) fn audit_pending(base: &Path, mount_id: &MountId) -> io::Result<usize> {
    audit_pending_state(base, mount_id)?.ok_or_else(|| io::Error::new(
        io::ErrorKind::WouldBlock,
        "mount_cache_inventory_unknown: Offlinezustand ist ohne Schlüssel oder Journalreparatur nicht zählbar; Cache bleibt erhalten"))
}

/// Strictly read-only inventory: no creation, rotation, truncation, orphan
/// cleanup or PlainPathSealer over ciphertext. None preserves unknown state.
pub(super) fn audit_pending_state(base: &Path, mount_id: &MountId) -> io::Result<Option<usize>> {
    use std::{
        ffi::OsStr,
        io::{BufReader, Read},
    };
    let base = match DirectoryHandle::open_root(base) {
        Ok(base) => base,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Some(0)),
        Err(error) => return Err(error),
    };
    let root = match base.open_child(OsStr::new(mount_id.as_str())) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Some(0)),
        Err(error) => return Err(error),
    };
    let mut journals = HashSet::new();
    for child in root.read_directory()? {
        let child = child?;
        match child.name.to_str() {
            Some("spool-key.wrapped") => return Ok(None),
            Some(
                name
                @ ("journal.jsonl" | "journal.jsonl.compact-old" | "journal.jsonl.compact-new"),
            ) => {
                if child.is_link_like || !matches!(child.kind, EntryKind::File) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Offlinejournal ist kein regulärer Eintrag; Cache bleibt unverändert",
                    ));
                }
                journals.insert(name.to_string());
            }
            _ => {}
        }
    }
    let selected = [
        "journal.jsonl",
        "journal.jsonl.compact-old",
        "journal.jsonl.compact-new",
    ]
    .into_iter()
    .find(|name| journals.contains(*name));
    let Some(selected) = selected else {
        match root.open_child(OsStr::new("files")) {
            Ok(files) if files.read_directory()?.next().is_some() => return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Recoveryjournal fehlt bei vorhandenen Cachedateien; alle Bytes bleiben erhalten",
            )),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        return Ok(Some(0));
    };
    let file = root.open_regular_child(OsStr::new(selected))?;
    if file.metadata()?.len() > audit::MAX_AUDIT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Offlinejournal überschreitet 64 MiB; Originalbytes bleiben erhalten",
        ));
    }
    audit::pending(BufReader::new(file.take(audit::MAX_AUDIT_BYTES + 1)))
}

impl Drop for WholeFileSpool {
    fn drop(&mut self) {
        if !self.cleanup_ready {
            return;
        }
        if let Ok(referenced) = self.referenced.lock() {
            let _ = self.remove_orphan_clean_files(&referenced);
        }
    }
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "mount spool path is not a plain directory: {}",
                    path.display()
                ),
            ));
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "mount spool directory was replaced by a link-like object",
        ));
    }
    Ok(())
}

fn validate_spool_name(name: &str) -> io::Result<()> {
    if !is_spool_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid mount spool file name",
        ));
    }
    Ok(())
}

fn is_spool_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 38 && &bytes[32..] == b".spool" && bytes[..32].iter().all(u8::is_ascii_hexdigit)
}

fn random_hex() -> io::Result<String> {
    let mut random = [0u8; 16];
    getrandom::getrandom(&mut random)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
    let mut value = String::with_capacity(32);
    for byte in random {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
    }
    Ok(value)
}

fn lock<T>(mutex: &Mutex<T>) -> io::Result<std::sync::MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "mount spool lock is poisoned"))
}
