use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::identity::{DirectCodeRotation, IdentityPersistence, ShareIdentity};

const IDENTITY_FILE: &str = "share_identity.json";
const MAX_IDENTITY_BYTES: u64 = 64 * 1024;

impl ShareIdentity {
    pub fn load_or_create(default_name: String) -> Result<Self, String> {
        Self::load_or_create_with(default_name, &mut SystemIdentityPersistence)
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_with(&mut SystemIdentityPersistence)
    }

    pub fn regenerate_direct_code(&mut self) -> Result<DirectCodeRotation, String> {
        self.regenerate_direct_code_with(&mut SystemIdentityPersistence)
    }

    pub fn set_device_name(&mut self, name: String) -> Result<(), String> {
        self.set_device_name_with(name, &mut SystemIdentityPersistence)
    }
}

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
