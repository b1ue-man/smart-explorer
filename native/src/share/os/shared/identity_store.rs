use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::identity::{
    DirectCodeRotation, IdentityPersistence, IdentityRepair, IdentityRepairAction, ShareIdentity,
};

const IDENTITY_FILE: &str = "share_identity.json";
const MAX_IDENTITY_BYTES: u64 = 64 * 1024;
static IDENTITY_TRANSACTION_LOCK: Mutex<()> = Mutex::new(());

struct IdentityTransactionGuard {
    _process_guard: MutexGuard<'static, ()>,
    _system_guard: super::identity_lock::IdentityLock,
}

impl ShareIdentity {
    pub fn load_or_create(default_name: String) -> Result<Self, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        load_ready_identity(default_name, Some(default_home()), &mut storage)
    }

    pub fn repair_missing(default_name: String) -> Result<IdentityRepair, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        let mut repair = Self::repair_missing_with(default_name, &mut storage)?;
        if let Err(error) =
            finish_pending_cleanup_locked(Some(default_home()), &repair.identity, &mut storage)
        {
            append_cleanup_warning(
                &mut repair.cleanup_warning,
                format!("Direkt-Freigaben nach Identitaetsreparatur sperren: {error}"),
            );
        }
        Ok(repair)
    }

    pub fn repair_action_needed(default_name: String) -> Result<IdentityRepairAction, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        Self::repair_action_needed_with(default_name, &mut SystemIdentityPersistence)
    }

    pub fn regenerate_direct_code(&mut self) -> Result<DirectCodeRotation, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        load_ready_identity(self.device_name.clone(), Some(default_home()), &mut storage)?;
        // The durable marker written by this mutation is completed by the GUI
        // while the worker remains stopped. A crash before that point is safe:
        // every production identity load completes the same cleanup first.
        self.regenerate_direct_code_with(&mut storage)
    }

    pub(crate) fn complete_pending_cleanup(
        &self,
        default_home: Option<String>,
    ) -> Result<super::profiles::ShareProfiles, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        let current = Self::load_or_create_with(self.device_name.clone(), &mut storage)?;
        with_matching_identity_generation(self, &current, |locked| {
            let load_home = default_home.clone();
            finish_pending_cleanup_locked(default_home, locked, &mut storage)?.map_or_else(
                || super::profiles::ShareProfiles::load_checked(load_home),
                Ok,
            )
        })
    }

    pub(crate) fn with_current_locked<T>(
        default_name: String,
        action: impl FnOnce(&ShareIdentity) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        let identity = load_ready_identity(default_name, Some(default_home()), &mut storage)?;
        action(&identity)
    }

    pub fn set_device_name(&mut self, name: String) -> Result<(), String> {
        let _guard = identity_transaction_guard()
            .map_err(|error| format!("Share-Identitaet sperren: {error}"))?;
        let mut storage = SystemIdentityPersistence;
        load_ready_identity(self.device_name.clone(), Some(default_home()), &mut storage)?;
        self.set_device_name_with(name, &mut storage)
    }
}

fn load_ready_identity(
    default_name: String,
    default_home: Option<String>,
    storage: &mut impl IdentityPersistence,
) -> Result<ShareIdentity, String> {
    let identity = ShareIdentity::load_or_create_with(default_name, storage)?;
    finish_pending_cleanup_locked(default_home, &identity, storage)?;
    Ok(identity)
}

fn finish_pending_cleanup_locked(
    default_home: Option<String>,
    identity: &ShareIdentity,
    storage: &mut impl IdentityPersistence,
) -> Result<Option<super::profiles::ShareProfiles>, String> {
    finish_pending_cleanup_with(identity, storage, |action| {
        super::legacy_direct_actions::invalidate_direct_grants_after_identity_rotation(
            default_home,
            identity,
            action,
        )
    })
}

fn finish_pending_cleanup_with<T>(
    identity: &ShareIdentity,
    storage: &mut impl IdentityPersistence,
    cleanup: impl FnOnce(IdentityRepairAction) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let Some(action) = ShareIdentity::pending_cleanup_action_with(storage)? else {
        return Ok(None);
    };
    let cleaned = cleanup(action)?;
    identity.clear_pending_cleanup_with(storage)?;
    Ok(Some(cleaned))
}

fn default_home() -> String {
    default_home_path().to_string_lossy().replace('\\', "/")
}

fn default_home_path() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

pub(crate) fn with_matching_identity_generation<T>(
    expected: &ShareIdentity,
    current: &ShareIdentity,
    action: impl FnOnce(&ShareIdentity) -> Result<T, String>,
) -> Result<T, String> {
    let matches = expected.device_id == current.device_id
        && expected.direct_lookup_id == current.direct_lookup_id
        && expected.public_key == current.public_key
        && expected.fingerprint == current.fingerprint
        && expected.node_id == current.node_id
        && expected.direct_secret == current.direct_secret;
    if !matches {
        return Err(
            "Share identity changed concurrently; reload the request and retry".to_string(),
        );
    }
    action(current)
}

fn append_cleanup_warning(target: &mut Option<String>, warning: String) {
    *target = Some(match target.take() {
        Some(existing) => format!("{existing}; {warning}"),
        None => warning,
    });
}

#[cfg(test)]
#[path = "identity_store_tests.rs"]
mod generation_tests;

struct SystemIdentityPersistence;

impl IdentityPersistence for SystemIdentityPersistence {
    fn load_identity(&mut self) -> Result<Option<String>, String> {
        load_identity().map_err(|error| error.to_string())
    }

    fn save_identity(&mut self, contents: &str) -> Result<(), String> {
        save_identity(contents).map_err(|error| error.to_string())
    }

    fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
        load_secret(account)
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        save_secret(account, secret)
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        delete_secret(account)
    }
}

pub(super) fn load_identity() -> io::Result<Option<String>> {
    let path = identity_path();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(invalid("Share identity is not a regular file"));
    }
    if metadata.len() > MAX_IDENTITY_BYTES {
        return Err(invalid("Share identity exceeds its byte budget"));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("Share identity size does not fit this platform"))?;
    let mut bytes = Vec::with_capacity(capacity);
    std::fs::File::open(path)?
        .take(MAX_IDENTITY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IDENTITY_BYTES {
        return Err(invalid("Share identity exceeds its byte budget"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| invalid("Share identity is not valid UTF-8"))
}

pub(super) fn save_identity(contents: &str) -> io::Result<()> {
    if contents.len() as u64 > MAX_IDENTITY_BYTES {
        return Err(invalid("Share identity exceeds its byte budget"));
    }
    let path = identity_path();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut staged = None;
    for attempt in 0..1000u32 {
        let candidate = path.with_extension(format!(
            "se-identity-{}-{nonce:x}-{attempt:x}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                let result = file
                    .write_all(contents.as_bytes())
                    .and_then(|()| file.flush())
                    .and_then(|()| file.sync_all());
                if let Err(error) = result {
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                staged = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let staged = staged.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate Share identity staging file",
        )
    })?;
    let staged_text = unicode_path(&staged)?;
    let path_text = unicode_path(&path)?;
    let backend = crate::vfs::LocalBackend::new("/");
    let result = crate::vfs::promote_staged_replace(&backend, staged_text, path_text);
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

pub(super) fn load_secret(account: &str) -> Result<Option<String>, String> {
    crate::creds::get_secret_checked(account)
}

pub(super) fn save_secret(account: &str, secret: &str) -> Result<(), String> {
    crate::creds::set_secret(account, secret)?;
    match crate::creds::get_secret_checked(account)? {
        Some(stored) if stored == secret => Ok(()),
        Some(_) => Err("secure store returned different Share secret bytes".into()),
        None => Err("secure store did not retain the Share secret".into()),
    }
}

pub(super) fn delete_secret(account: &str) -> Result<(), String> {
    crate::creds::delete_secret_checked(account)
}

fn identity_transaction_guard() -> io::Result<IdentityTransactionGuard> {
    let process_guard = match IDENTITY_TRANSACTION_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let system_guard = super::identity_lock::acquire(&crate::support_dirs::app_data_dir())?;
    Ok(IdentityTransactionGuard {
        _process_guard: process_guard,
        _system_guard: system_guard,
    })
}

fn identity_path() -> PathBuf {
    crate::support_dirs::app_data_file(IDENTITY_FILE)
}

fn unicode_path(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("Share identity path is not valid Unicode"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
