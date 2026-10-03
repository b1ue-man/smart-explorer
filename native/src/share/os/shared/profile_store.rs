use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fmt, str};

#[path = "profile_transaction.rs"]
mod profile_transaction;

use super::profile_persistence::{
    encode_profiles, ProfileChange, ProfilePersistence, MAX_PROFILE_BYTES,
};
use super::profiles::{
    direct_contact_secret_account, room_secret_account, ProfileRevision, ShareProfiles,
    SHARE_PROFILE_VERSION,
};
use super::room_relation::RoomRelationMaterial;
use super::types::{DirectContact, DirectGrantState, PeerPresence, RoomProfile};

const PROFILES_FILE: &str = "share_profiles.json";
const SECRET_BYTES: usize = 32;
const PROFILES_LOCK_FILE: &str = "share_profiles.lock";
static PROFILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// UTF-8 credential material whose owned buffer is redacted in diagnostics and
/// erased when it leaves scope.
pub(super) struct SecretString {
    bytes: Vec<u8>,
}

impl SecretString {
    pub(super) fn from_string(value: String) -> Self {
        Self {
            bytes: value.into_bytes(),
        }
    }

    pub(super) fn encoded(value: &[u8]) -> Self {
        Self::from_string(super::core::b64(value))
    }

    pub(super) fn expose(&self) -> Result<&str, String> {
        str::from_utf8(&self.bytes).map_err(|_| "credential secret is not valid UTF-8".to_string())
    }

    pub(super) fn same_secret(&self, other: &Self) -> bool {
        constant_time_eq(&self.bytes, &other.bytes)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let common = left.len().min(right.len());
    for index in 0..common {
        difference |= usize::from(left[index] ^ right[index]);
    }
    difference == 0
}

pub(super) fn credential_matches(account: &str, expected: &SecretString) -> Result<bool, String> {
    let stored = crate::creds::get_secret_checked(account)?;
    Ok(stored
        .map(SecretString::from_string)
        .is_some_and(|stored| stored.same_secret(expected)))
}

/// Remove a credential and confirm absence. Verification makes a backend that
/// reports an error after successfully deleting safe to retry.
pub(super) fn delete_credential_verified(account: &str) -> Result<(), String> {
    let mut last_error = "secure credential remained after deletion".to_string();
    for _ in 0..3 {
        let deletion = crate::creds::delete_secret_checked(account);
        match crate::creds::get_secret_checked(account) {
            Ok(None) => return Ok(()),
            Ok(Some(stored)) => {
                let _stored = SecretString::from_string(stored);
                last_error = match deletion {
                    Ok(()) => "secure credential remained after deletion".to_string(),
                    Err(error) => error,
                };
            }
            Err(verification) => {
                last_error = match deletion {
                    Ok(()) => format!(
                        "secure credential deletion could not be verified: {verification}"
                    ),
                    Err(deletion) => format!(
                        "secure credential deletion failed ({deletion}) and could not be verified ({verification})"
                    ),
                };
            }
        }
    }
    Err(last_error)
}

/// Install a secret only into an unused credential account and verify the
/// exact bytes. Any failure after the write attempts verified cleanup.
pub(super) fn prepare_unique_credential(
    account: &str,
    secret: &SecretString,
) -> Result<(), String> {
    match crate::creds::get_secret_checked(account)? {
        None => {}
        Some(existing) => {
            let _existing = SecretString::from_string(existing);
            return Err("secure credential account is already occupied".to_string());
        }
    }
    let operation = secret.expose().and_then(|exposed| {
        crate::creds::set_secret(account, exposed)?;
        if credential_matches(account, secret)? {
            Ok(())
        } else {
            Err("secure store did not retain the expected credential bytes".to_string())
        }
    });
    if let Err(operation) = operation {
        return match delete_credential_verified(account) {
            Ok(()) => Err(operation),
            Err(cleanup) => Err(format!(
                "{operation}; prepared credential cleanup failed: {cleanup}"
            )),
        };
    }
    Ok(())
}

struct ProfileWriteGuard {
    _process_guard: MutexGuard<'static, ()>,
    _file_guard: std::fs::File,
}

impl ShareProfiles {
    pub fn load_checked(default_home: Option<String>) -> Result<Self, String> {
        let mut storage = SystemProfilePersistence::default();
        for _ in 0..5 {
            storage.save_conflict = false;
            match Self::load_checked_with(default_home.clone(), &mut storage) {
                Err(_) if storage.save_conflict => continue,
                result => return result,
            }
        }
        Err("Share-Profile wurden waehrend der Migration wiederholt geaendert; bitte erneut versuchen".into())
    }

    pub fn load(default_home: Option<String>) -> Self {
        Self::load_checked(default_home).unwrap_or_else(|_| Self {
            storage_revision: ProfileRevision::Missing,
            ..Self::default()
        })
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.save_with(&mut SystemProfilePersistence::default())
    }

    pub fn persist_replacement(&mut self, candidate: ShareProfiles) -> Result<(), String> {
        self.persist_replacement_with(candidate, &mut SystemProfilePersistence::default())
    }

    /// Reapply an idempotent, field-level mutation to the latest persisted
    /// profile and return the canonical profile committed by this transaction.
    ///
    /// Another process can win the optimistic compare-and-swap between load
    /// and save. In that case `mutation` runs again against the newer profile,
    /// so it must not perform external side effects or depend on being called
    /// exactly once. Mutation and precondition errors are returned immediately.
    pub fn mutate_persisted<F>(
        default_home: Option<String>,
        mutation: F,
    ) -> Result<ShareProfiles, String>
    where
        F: FnMut(&mut ShareProfiles) -> Result<(), String>,
    {
        profile_transaction::run(
            || Self::load_checked(default_home.clone()),
            mutation,
            commit_transaction_candidate,
        )
        .map_err(|error| error.to_string())
    }

    pub fn add_direct_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        self.add_direct_from_code_with(code, name, &mut SystemProfilePersistence::default())
    }

    pub fn set_direct_grant_persisted(
        &mut self,
        presence: &PeerPresence,
        state: DirectGrantState,
    ) -> Result<(), String> {
        self.set_direct_grant_persisted_with(
            presence,
            state,
            &mut SystemProfilePersistence::default(),
        )
    }

    pub fn remove_direct_contact(&mut self, contact_id: &str) -> Result<ProfileChange, String> {
        self.remove_direct_contact_with(contact_id, &mut SystemProfilePersistence::default())
    }

    pub fn add_room_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        self.add_room_from_code_with(code, name, &mut SystemProfilePersistence::default())
    }

    pub fn remove_room(&mut self, room_id: &str) -> Result<ProfileChange, String> {
        self.remove_room_with(room_id, &mut SystemProfilePersistence::default())
    }

    pub fn direct_secret_checked(contact: &DirectContact) -> Result<Option<Vec<u8>>, String> {
        load_relation_secret(&direct_contact_secret_account(&contact.id), "Direkt-Secret")
    }

    pub fn direct_secret(contact: &DirectContact) -> Option<Vec<u8>> {
        Self::direct_secret_checked(contact).ok().flatten()
    }

    pub fn room_secret_checked(room: &RoomProfile) -> Result<Option<Vec<u8>>, String> {
        load_relation_secret(&room_secret_account(&room.id), "Raum-Secret")
    }

    pub fn room_secret(room: &RoomProfile) -> Option<Vec<u8>> {
        Self::room_secret_checked(room).ok().flatten()
    }

    pub fn room_relation_material_checked(
        room: &RoomProfile,
    ) -> Result<Option<RoomRelationMaterial>, String> {
        Self::room_secret_checked(room)?
            .map(|secret| {
                RoomRelationMaterial::new(room.room_id.clone(), secret)
                    .map_err(|error| error.to_string())
            })
            .transpose()
    }

    pub fn room_code_checked(room: &RoomProfile) -> Result<Option<String>, String> {
        Ok(Self::room_relation_material_checked(room)?.map(|material| {
            format!(
                "SE-R3-{}-{}",
                material.room_id(),
                super::core::hex(material.secret())
            )
        }))
    }

    pub fn room_code(room: &RoomProfile) -> Option<String> {
        Self::room_code_checked(room).ok().flatten()
    }
}

#[derive(Default)]
struct SystemProfilePersistence {
    save_conflict: bool,
}

impl ProfilePersistence for SystemProfilePersistence {
    fn saved_connection_accounts(&mut self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(
            crate::creds::load_connections_checked()?
                .iter()
                .map(|connection| connection.account())
                .collect(),
        ))
    }

    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        load_profiles().map_err(|error| error.to_string())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        match save_profiles(contents, expected) {
            Ok(revision) => {
                self.save_conflict = false;
                Ok(revision)
            }
            Err(error) => {
                self.save_conflict = error.kind() == io::ErrorKind::WouldBlock;
                Err(error.to_string())
            }
        }
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        let secret = SecretString::from_string(secret.to_string());
        prepare_unique_credential(account, &secret)
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        delete_credential_verified(account)
    }
}

fn commit_transaction_candidate(
    candidate: &mut ShareProfiles,
) -> Result<(), profile_transaction::CommitError> {
    candidate.schema_version = SHARE_PROFILE_VERSION;
    candidate.reconcile_legacy_grants(super::core::now_secs());
    candidate.recompute_all_identity_conflicts();
    candidate.validate_direct_ledger().map_err(|error| {
        profile_transaction::CommitError::Fatal(format!("Share-Profile sind beschaedigt: {error}"))
    })?;
    candidate
        .validate_legacy_direct_requests()
        .map_err(|error| {
            profile_transaction::CommitError::Fatal(format!(
                "Share-Profile sind beschaedigt: {error}"
            ))
        })?;
    let contents = encode_profiles(candidate).map_err(profile_transaction::CommitError::Fatal)?;
    match save_profiles(&contents, &candidate.storage_revision) {
        Ok(revision) => {
            candidate.storage_revision = revision;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err(profile_transaction::CommitError::Conflict)
        }
        Err(error) => Err(profile_transaction::CommitError::Fatal(format!(
            "Share-Profile speichern: {error}"
        ))),
    }
}

fn load_relation_secret(account: &str, label: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(raw) = crate::creds::get_secret_checked(account)? else {
        return Ok(None);
    };
    let raw = SecretString::from_string(raw);
    let mut decoded = super::core::b64_decode(raw.expose()?)
        .map_err(|error| format!("{label} ist ungueltig: {error}"))?;
    if decoded.len() != SECRET_BYTES {
        decoded.fill(0);
        return Err(format!("{label} hat nicht {SECRET_BYTES} Bytes"));
    }
    Ok(Some(decoded))
}

fn load_profiles() -> io::Result<Option<String>> {
    let path = profiles_path();
    let file = match crate::support_dirs::open_private_file(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if metadata.len() > MAX_PROFILE_BYTES {
        return Err(invalid("Share profiles exceed their byte budget"));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("Share profile size does not fit this platform"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(invalid("Share profiles exceed their byte budget"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| invalid("Share profiles are not valid UTF-8"))
}

fn save_profiles(contents: &str, expected: &ProfileRevision) -> io::Result<ProfileRevision> {
    if contents.len() as u64 > MAX_PROFILE_BYTES {
        return Err(invalid("Share profiles exceed their byte budget"));
    }
    let _guard = profile_write_guard()?;
    let current = load_profiles()?
        .as_deref()
        .map(ProfileRevision::from_contents)
        .unwrap_or(ProfileRevision::Missing);
    let matches_current = match expected {
        ProfileRevision::Untracked => current == ProfileRevision::Missing,
        revision => revision == &current,
    };
    if !matches_current {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Share profiles changed concurrently; reload and retry",
        ));
    }
    let path = profiles_path();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    crate::support_dirs::ensure_private_dir(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut staged = None;
    for attempt in 0..1000u32 {
        let candidate = path.with_extension(format!(
            "se-profiles-{}-{nonce:x}-{attempt:x}.tmp",
            std::process::id()
        ));
        match crate::support_dirs::create_private_file(&candidate) {
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
            "could not allocate Share profile staging file",
        )
    })?;
    let backend = crate::vfs::LocalBackend::new("/");
    let result =
        crate::vfs::promote_staged_replace(&backend, unicode_path(&staged)?, unicode_path(&path)?);
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result?;
    Ok(ProfileRevision::from_contents(contents))
}

fn profile_write_guard() -> io::Result<ProfileWriteGuard> {
    let process_guard = match PROFILE_WRITE_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let directory = crate::support_dirs::app_data_dir();
    crate::support_dirs::ensure_private_dir(&directory)?;
    let path = directory.join(PROFILES_LOCK_FILE);
    let file_guard = crate::support_dirs::open_private_lock(&path)?;
    file_guard.lock()?;
    Ok(ProfileWriteGuard {
        _process_guard: process_guard,
        _file_guard: file_guard,
    })
}

fn profiles_path() -> PathBuf {
    crate::support_dirs::app_data_file(PROFILES_FILE)
}

fn unicode_path(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| invalid("Share profile path is not valid Unicode"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
