use super::GDriveBackend;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const RECORD_VERSION: u32 = 1;
const RECORD_ROOT: &str = "pending-folder-creates";
const RECORD_FILE: &str = "record.json";
const STAGED_RECORD_FILE: &str = "record.tmp";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PendingFolderCreate {
    pub(super) account_key: String,
    pub(super) key: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) parent_id: String,
}

#[derive(Deserialize, Serialize)]
struct DiskRecord {
    version: u32,
    create: PendingFolderCreate,
}

pub(super) fn record_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("gdrive")
        .join(RECORD_ROOT)
}

pub(super) fn account_key(stable_permission_id: &str) -> String {
    use sha2::{Digest, Sha256};

    Sha256::digest(stable_permission_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl GDriveBackend {
    pub(super) fn pending_folder_create(
        &self,
        key: &str,
    ) -> io::Result<Option<PendingFolderCreate>> {
        if let Some(create) = self.pending_folder_creates_guard()?.get(key).cloned() {
            return Ok(Some(create));
        }
        let Some(root) = self.pending_folder_dir.as_deref() else {
            return Ok(None);
        };
        let Some(create) = load(root, &self.drive_account_key, key)? else {
            return Ok(None);
        };
        self.pending_folder_creates_guard()?
            .insert(key.to_string(), create.clone());
        Ok(Some(create))
    }

    pub(super) fn reserve_pending_folder_create(
        &self,
        key: &str,
        id: &str,
        name: &str,
        parent_id: &str,
    ) -> io::Result<(PendingFolderCreate, bool)> {
        if let Some(create) = self.pending_folder_create(key)? {
            return Ok((create, false));
        }
        let proposed = PendingFolderCreate {
            account_key: self.drive_account_key.to_string(),
            key: key.to_string(),
            id: id.to_string(),
            name: name.to_string(),
            parent_id: parent_id.to_string(),
        };
        let (create, claimed) = match self.pending_folder_dir.as_deref() {
            Some(root) => claim(root, &proposed)?,
            None => (proposed, true),
        };
        self.pending_folder_creates_guard()?
            .insert(key.to_string(), create.clone());
        Ok((create, claimed))
    }

    pub(super) fn clear_pending_folder_create(
        &self,
        expected: &PendingFolderCreate,
    ) -> io::Result<()> {
        if let Some(root) = self.pending_folder_dir.as_deref() {
            clear(root, expected)?;
        }
        let mut pending = self.pending_folder_creates_guard()?;
        if pending
            .get(&expected.key)
            .is_some_and(|current| current == expected)
        {
            pending.remove(&expected.key);
        }
        Ok(())
    }
}

fn generation_key(account_key: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(account_key.as_bytes());
    digest.update([0]);
    digest.update(key.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn active_dir(root: &Path, generation: &str) -> PathBuf {
    root.join(format!("{generation}.pending"))
}

fn resolved_prefix(generation: &str) -> String {
    format!("{generation}.resolved-")
}

fn load(root: &Path, account_key: &str, key: &str) -> io::Result<Option<PendingFolderCreate>> {
    load_generations(root, account_key, key, true)
}

fn load_resolved(
    root: &Path,
    account_key: &str,
    key: &str,
) -> io::Result<Option<PendingFolderCreate>> {
    load_generations(root, account_key, key, false)
}

fn load_generations(
    root: &Path,
    account_key: &str,
    key: &str,
    include_active: bool,
) -> io::Result<Option<PendingFolderCreate>> {
    let generation = generation_key(account_key, key);
    let active_name = format!("{generation}.pending");
    let resolved_prefix = resolved_prefix(&generation);
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut found: Option<PendingFolderCreate> = None;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let matches = (include_active && name == active_name) || name.starts_with(&resolved_prefix);
        if !matches {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive folder journal generation is not a directory",
            ));
        }
        let create = load_generation(&entry.path(), account_key, key)?;
        if found.as_ref().is_some_and(|current| current != &create) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive folder journal contains conflicting generations",
            ));
        }
        found = Some(create);
    }
    Ok(found)
}

fn load_generation(
    directory: &Path,
    account_key: &str,
    key: &str,
) -> io::Result<PendingFolderCreate> {
    let text = std::fs::read_to_string(directory.join(RECORD_FILE)).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Drive folder journal generation is incomplete: {error}"),
        )
    })?;
    let record: DiskRecord = serde_json::from_str(&text).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Drive folder journal record is invalid: {error}"),
        )
    })?;
    if record.version != RECORD_VERSION
        || record.create.account_key != account_key
        || record.create.key != key
        || record.create.id.is_empty()
        || record.create.name.is_empty()
        || record.create.parent_id.is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive folder journal record does not match its account and path",
        ));
    }
    Ok(record.create)
}

fn claim(root: &Path, proposed: &PendingFolderCreate) -> io::Result<(PendingFolderCreate, bool)> {
    std::fs::create_dir_all(root)?;
    if let Some(existing) = load(root, &proposed.account_key, &proposed.key)? {
        return Ok((existing, false));
    }
    let generation = generation_key(&proposed.account_key, &proposed.key);
    let active = active_dir(root, &generation);
    match std::fs::create_dir(&active) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = load(root, &proposed.account_key, &proposed.key)?.ok_or_else(|| {
                io::Error::other("Drive folder reservation disappeared during claim")
            })?;
            return Ok((existing, false));
        }
        Err(error) => return Err(error),
    }

    // Close the scan/create race with a concurrent resolver: if an atomically
    // renamed generation exists, release our still-empty claim and use it.
    match load_resolved(root, &proposed.account_key, &proposed.key) {
        Ok(Some(existing)) => {
            std::fs::remove_dir(&active)?;
            return Ok((existing, false));
        }
        Ok(None) => {}
        Err(error) => {
            let _ = std::fs::remove_dir(&active);
            return Err(error);
        }
    }
    write_generation(&active, proposed)?;
    Ok((proposed.clone(), true))
}

fn write_generation(directory: &Path, create: &PendingFolderCreate) -> io::Result<()> {
    let staged = directory.join(STAGED_RECORD_FILE);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    serde_json::to_writer_pretty(
        &mut file,
        &DiskRecord {
            version: RECORD_VERSION,
            create: create.clone(),
        },
    )
    .map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(staged, directory.join(RECORD_FILE))
}

fn clear(root: &Path, expected: &PendingFolderCreate) -> io::Result<()> {
    let generation = generation_key(&expected.account_key, &expected.key);
    let active = active_dir(root, &generation);
    let tombstone = unique_tombstone(root, &generation)?;
    match std::fs::rename(&active, &tombstone) {
        Ok(()) => clear_generation(&tombstone, expected)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    // A crash after the rename leaves a discoverable resolved generation. A
    // later exact-ID reconciliation clears it here without touching any newer
    // active generation at the original path.
    let prefix = resolved_prefix(&generation);
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            clear_generation(&entry.path(), expected)?;
        }
    }
    Ok(())
}

fn clear_generation(directory: &Path, expected: &PendingFolderCreate) -> io::Result<()> {
    let moved = load_generation(directory, &expected.account_key, &expected.key)?;
    if &moved != expected {
        return Err(io::Error::other(
            "refusing to clear a different Drive folder reservation generation",
        ));
    }
    let mut saw_record = false;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_name() != RECORD_FILE || !entry.file_type()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive folder journal generation contains an unexpected entry",
            ));
        }
        saw_record = true;
    }
    if !saw_record {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive folder journal generation has no record",
        ));
    }
    // Remove only the validated leaf and then the now-empty directory. Never
    // recurse through an attacker-controlled link/reparse child.
    std::fs::remove_file(directory.join(RECORD_FILE))?;
    std::fs::remove_dir(directory)
}

fn unique_tombstone(root: &Path, generation: &str) -> io::Result<PathBuf> {
    static NEXT_TOMBSTONE: AtomicU64 = AtomicU64::new(0);

    for _ in 0..1_000 {
        let nonce = NEXT_TOMBSTONE.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(
            "{}.resolved-{}-{nonce:016x}",
            generation,
            std::process::id()
        ));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate Drive folder-journal tombstone",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(id: &str) -> PendingFolderCreate {
        PendingFolderCreate {
            account_key: account_key("stable-permission-id"),
            key: "folder".to_string(),
            id: id.to_string(),
            name: "folder".to_string(),
            parent_id: "root".to_string(),
        }
    }

    #[test]
    fn crash_after_clear_rename_is_discovered_before_any_new_claim() {
        let storage = tempfile::tempdir().unwrap();
        let original = create("reserved-id");
        assert_eq!(
            claim(storage.path(), &original).unwrap(),
            (original.clone(), true)
        );

        // Model a process dying immediately after the atomic clear rename and
        // before it can validate/delete the moved generation.
        let generation = generation_key(&original.account_key, &original.key);
        let active = active_dir(storage.path(), &generation);
        let tombstone = unique_tombstone(storage.path(), &generation).unwrap();
        std::fs::rename(active, &tombstone).unwrap();

        assert_eq!(
            load(storage.path(), &original.account_key, &original.key).unwrap(),
            Some(original.clone())
        );
        let replacement = create("must-not-be-claimed");
        assert_eq!(
            claim(storage.path(), &replacement).unwrap(),
            (original.clone(), false)
        );
        assert!(!active_dir(storage.path(), &generation).exists());

        clear(storage.path(), &original).unwrap();
        assert_eq!(
            load(storage.path(), &original.account_key, &original.key).unwrap(),
            None
        );
    }

    #[test]
    fn clear_fails_closed_without_traversing_an_unexpected_child() {
        let storage = tempfile::tempdir().unwrap();
        let original = create("reserved-id");
        claim(storage.path(), &original).unwrap();
        let generation = generation_key(&original.account_key, &original.key);
        let unexpected = active_dir(storage.path(), &generation).join("unexpected");
        std::fs::create_dir(&unexpected).unwrap();
        let sentinel = unexpected.join("sentinel");
        std::fs::write(&sentinel, b"keep").unwrap();

        let error = clear(storage.path(), &original).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let resolved = std::fs::read_dir(storage.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".resolved-")
            })
            .unwrap();
        assert_eq!(
            std::fs::read(resolved.join("unexpected/sentinel")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn stale_clear_cannot_delete_a_newer_generation() {
        let storage = tempfile::tempdir().unwrap();
        let old = create("old-id");
        claim(storage.path(), &old).unwrap();
        clear(storage.path(), &old).unwrap();

        let new = create("new-id");
        assert_eq!(claim(storage.path(), &new).unwrap(), (new.clone(), true));
        let error = clear(storage.path(), &old).unwrap_err();
        assert!(error.to_string().contains("different"));
        assert_eq!(
            load(storage.path(), &new.account_key, &new.key).unwrap(),
            Some(new)
        );
    }
}
