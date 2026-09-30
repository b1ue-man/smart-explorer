use super::journal::{Journal, PersistedDelete, PersistedEntry, RecoveredJournal};
use super::types::{MountId, NamespaceIntent};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const ALLOCATION_ATTEMPTS: usize = 128;

#[path = "spool_recovery.rs"]
mod recovery;

pub(super) struct AllocatedSpool {
    pub name: String,
    pub file: File,
}

pub(super) struct WholeFileSpool {
    files: PathBuf,
    journal: Mutex<Journal>,
    referenced: Mutex<HashSet<String>>,
    missing_recovery: HashSet<String>,
    cleanup_ready: bool,
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
    pub fn open(base: &Path, mount_id: &MountId) -> io::Result<(Self, RecoveredJournal)> {
        ensure_directory(base)?;
        let root = base.join(mount_id.as_str());
        ensure_directory(&root)?;
        let files = root.join("files");
        ensure_directory(&files)?;
        recovery::require_journal_for_existing_files(&root, &files)?;
        let (journal, mut recovered) = Journal::open(&root.join("journal.jsonl"))?;
        let mut spool = Self {
            files,
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
        };
        let mut names = HashSet::new();
        for entry in recovered.entries.values_mut() {
            if !names.insert(entry.spool_name.clone()) {
                return Err(io::Error::new(io::ErrorKind::InvalidData,
                    "two recovery entries reference the same mount spool"));
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

    pub fn allocate(&self) -> io::Result<AllocatedSpool> {
        for _ in 0..ALLOCATION_ATTEMPTS {
            let name = format!("{}.spool", random_hex()?);
            let path = self.files.join(&name);
            match OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)
            {
                Ok(file) => return Ok(AllocatedSpool { name, file }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique mount spool file",
        ))
    }

    pub fn open_file(&self, name: &str, writable: bool) -> io::Result<File> {
        validate_spool_name(name)?;
        let path = self.files.join(name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| recovery::file_error(&path, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount spool entry is not a plain file",
            ));
        }
        OpenOptions::new().read(true).write(writable).open(&path)
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
                Err(error) => { first_error.get_or_insert(error); continue; }
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
    let (_spool, recovered) = WholeFileSpool::open(base, mount_id)?;
    if recovered.entries.is_empty()
        && recovered.deletes.is_empty()
        && recovered.namespace_conflicts.is_empty()
    {
        Ok(super::recovery_state::MountRecovery::Clean)
    } else {
        Ok(super::recovery_state::MountRecovery::Required)
    }
}

impl Drop for WholeFileSpool {
    fn drop(&mut self) {
        if !self.cleanup_ready { return; }
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
