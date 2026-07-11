use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::profile_persistence::{ProfileChange, ProfilePersistence};
use super::profiles::{
    direct_contact_secret_account, room_secret_account, ProfileRevision, ShareProfiles,
};
use super::types::{DirectContact, DirectGrantState, PeerPresence, RoomProfile};

const PROFILES_FILE: &str = "share_profiles.json";
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
const SECRET_BYTES: usize = 32;
const PROFILES_LOCK_FILE: &str = "share_profiles.lock";
static PROFILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

struct ProfileWriteGuard {
    _process_guard: MutexGuard<'static, ()>,
    _file_guard: std::fs::File,
}

impl ShareProfiles {
    pub fn load_checked(default_home: Option<String>) -> Result<Self, String> {
        Self::load_checked_with(default_home, &mut SystemProfilePersistence)
    }

    pub fn load(default_home: Option<String>) -> Self {
        Self::load_checked(default_home).unwrap_or_default()
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.save_with(&mut SystemProfilePersistence)
    }

    pub fn persist_replacement(&mut self, candidate: ShareProfiles) -> Result<(), String> {
        self.persist_replacement_with(candidate, &mut SystemProfilePersistence)
    }

    pub fn add_direct_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        self.add_direct_from_code_with(code, name, &mut SystemProfilePersistence)
    }

    pub fn set_direct_grant_persisted(
        &mut self,
        presence: &PeerPresence,
        state: DirectGrantState,
    ) -> Result<(), String> {
        self.set_direct_grant_persisted_with(presence, state, &mut SystemProfilePersistence)
    }

    pub fn remove_direct_contact(&mut self, contact_id: &str) -> Result<ProfileChange, String> {
        self.remove_direct_contact_with(contact_id, &mut SystemProfilePersistence)
    }

    pub fn add_room_from_code(&mut self, code: &str, name: &str) -> Result<String, String> {
        self.add_room_from_code_with(code, name, &mut SystemProfilePersistence)
    }

    pub fn remove_room(&mut self, room_id: &str) -> Result<ProfileChange, String> {
        self.remove_room_with(room_id, &mut SystemProfilePersistence)
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

    pub fn room_code_checked(room: &RoomProfile) -> Result<Option<String>, String> {
        Ok(Self::room_secret_checked(room)?
            .map(|secret| format!("SE-R3-{}-{}", room.room_id, super::core::hex(&secret))))
    }

    pub fn room_code(room: &RoomProfile) -> Option<String> {
        Self::room_code_checked(room).ok().flatten()
    }
}

struct SystemProfilePersistence;

impl ProfilePersistence for SystemProfilePersistence {
    fn load_profiles(&mut self) -> Result<Option<String>, String> {
        load_profiles().map_err(|error| error.to_string())
    }

    fn save_profiles(
        &mut self,
        contents: &str,
        expected: &ProfileRevision,
    ) -> Result<ProfileRevision, String> {
        save_profiles(contents, expected).map_err(|error| error.to_string())
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        crate::creds::set_secret(account, secret)?;
        match crate::creds::get_secret_checked(account)? {
            Some(stored) if stored == secret => Ok(()),
            Some(_) => Err("secure store returned different Share secret bytes".into()),
            None => Err("secure store did not retain the Share secret".into()),
        }
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        crate::creds::delete_secret_checked(account)
    }
}

fn load_relation_secret(account: &str, label: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(raw) = crate::creds::get_secret_checked(account)? else {
        return Ok(None);
    };
    let decoded =
        super::core::b64_decode(&raw).map_err(|error| format!("{label} ist ungueltig: {error}"))?;
    if decoded.len() != SECRET_BYTES {
        return Err(format!("{label} hat nicht {SECRET_BYTES} Bytes"));
    }
    Ok(Some(decoded))
}

fn load_profiles() -> io::Result<Option<String>> {
    let path = profiles_path();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() {
        return Err(invalid("Share profiles are not a regular file"));
    }
    if metadata.len() > MAX_PROFILE_BYTES {
        return Err(invalid("Share profiles exceed their byte budget"));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("Share profile size does not fit this platform"))?;
    let mut bytes = Vec::with_capacity(capacity);
    std::fs::File::open(path)?
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
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
    if !matches!(expected, ProfileRevision::Untracked) && expected != &current {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Share profiles changed concurrently; reload and retry",
        ));
    }
    let path = profiles_path();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
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
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(PROFILES_LOCK_FILE);
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Share profile transaction lock is not a regular file",
            ));
        }
    }
    let file_guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
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
