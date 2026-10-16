//! Who protects the data key of the file credential store (Linux, Android).
//!
//! The protector is chosen once, when a store first needs a data key, and is
//! recorded in `secrets-v1/keyring-v2`:
//! 1. `android-keystore`: the app unwrapped the key with a non-exportable
//!    Android Keystore AES key and handed it in at `init`.
//! 2. `systemd-creds-system` / `systemd-creds-user` (Linux): `systemd-creds
//!    --with-key=host`, system scope for root, user scope (systemd ≥ 256) for
//!    everybody else. The host secret belongs to root, so a copy of the home
//!    directory or of the app data alone does not open the store. No TPM:
//!    a cleared TPM (firmware update) would destroy every stored secret.
//! 3. `app-key-file`: a key file next to the store directory. It only keeps
//!    the store directory alone from opening; offline secrecy then needs disk
//!    encryption, which `se doctor` and the docs say.
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use zeroize::Zeroizing;

use crate::creds::sealed_record::{unwrap_key, wrap_key, DataKey, KeyringDescriptor, KEY_BYTES};

pub(crate) const ANDROID_KEYSTORE: &str = "android-keystore";
pub(crate) const SYSTEMD_CREDS_SYSTEM: &str = "systemd-creds-system";
pub(crate) const SYSTEMD_CREDS_USER: &str = "systemd-creds-user";
pub(crate) const APP_KEY_FILE: &str = "app-key-file";

static HOST_DATA_KEY: Mutex<Option<DataKey>> = Mutex::new(None);

/// Android: the data key the app unwrapped from the Android Keystore. Only
/// the first key counts; a different later key is refused.
pub(crate) fn install_host_data_key(bytes: &[u8]) -> Result<(), String> {
    let key = DataKey::from_slice(bytes)?;
    let mut slot = HOST_DATA_KEY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match slot.as_ref() {
        Some(existing) if existing.id() == key.id() => Ok(()),
        Some(_) => Err("credential store: a different host data key is already installed".into()),
        None => {
            *slot = Some(key);
            Ok(())
        }
    }
}

fn host_data_key() -> Option<DataKey> {
    HOST_DATA_KEY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// A data key protected for one store: the protector name and its
/// protector-specific payload for the descriptor.
pub(crate) struct Protected {
    pub(crate) protector: &'static str,
    pub(crate) wrapped: String,
}

pub(crate) trait KeyProtector: Send + Sync {
    /// Data key for a store that has none yet.
    fn new_key(&self) -> Result<DataKey, String>;
    /// Protects `key` for the store in `directory`.
    fn protect(&self, directory: &Path, key: &DataKey) -> Result<Protected, String>;
    /// Recovers the key a descriptor of `directory` names.
    fn recover(&self, directory: &Path, descriptor: &KeyringDescriptor) -> Result<DataKey, String>;
}

/// The production choice described in the module documentation.
pub(crate) struct PlatformProtector;

impl KeyProtector for PlatformProtector {
    fn new_key(&self) -> Result<DataKey, String> {
        match host_data_key() {
            Some(key) => Ok(key),
            None => DataKey::generate(),
        }
    }

    fn protect(&self, directory: &Path, key: &DataKey) -> Result<Protected, String> {
        if host_data_key().is_some_and(|host| host.id() == key.id()) {
            return Ok(Protected {
                protector: ANDROID_KEYSTORE,
                wrapped: String::new(),
            });
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(protected) = super::systemd_creds::protect(directory, key) {
                return Ok(protected);
            }
        }
        protect_with_key_file(directory, key)
    }

    fn recover(&self, directory: &Path, descriptor: &KeyringDescriptor) -> Result<DataKey, String> {
        let key = match descriptor.protector.as_str() {
            ANDROID_KEYSTORE => host_data_key().ok_or_else(|| {
                "Geheimnisspeicher gesperrt: der Android-Schlüsselspeicher hat beim Start keinen Schlüssel geliefert"
                    .to_string()
            })?,
            #[cfg(target_os = "linux")]
            SYSTEMD_CREDS_SYSTEM | SYSTEMD_CREDS_USER => {
                super::systemd_creds::recover(descriptor)?
            }
            APP_KEY_FILE => recover_with_key_file(directory, descriptor)?,
            other => {
                return Err(format!(
                    "Geheimnisspeicher: Schlüsselhüter {other:?} ist auf diesem System nicht verfügbar"
                ))
            }
        };
        check_id(&key, descriptor)?;
        Ok(key)
    }
}

pub(crate) fn check_id(key: &DataKey, descriptor: &KeyringDescriptor) -> Result<(), String> {
    if key.id_hex() == descriptor.key_id {
        Ok(())
    } else {
        Err(
            "Geheimnisspeicher gesperrt: der gelieferte Schlüssel passt nicht zu diesem Speicher"
                .into(),
        )
    }
}

/// `<store>.kek`, for example `secrets-v1.kek` beside `secrets-v1/`.
pub(crate) fn key_file_path(directory: &Path) -> PathBuf {
    directory.with_extension("kek")
}

pub(crate) fn protect_with_key_file(directory: &Path, key: &DataKey) -> Result<Protected, String> {
    let kek = load_or_create_key_file(&key_file_path(directory))?;
    let wrapped = wrap_key(&kek, key)?;
    Ok(Protected {
        protector: APP_KEY_FILE,
        wrapped: base64::engine::general_purpose::STANDARD.encode(wrapped),
    })
}

fn recover_with_key_file(
    directory: &Path,
    descriptor: &KeyringDescriptor,
) -> Result<DataKey, String> {
    let kek = read_key_file(&key_file_path(directory))?
        .ok_or_else(|| "Geheimnisspeicher gesperrt: die Schlüsseldatei fehlt".to_string())?;
    let wrapped = base64::engine::general_purpose::STANDARD
        .decode(descriptor.wrapped.as_bytes())
        .map_err(|_| "Geheimnisspeicher: verpackter Schlüssel ist kein Base64".to_string())?;
    unwrap_key(&kek, &wrapped)
}

/// Called under the store lock. The key file appears complete or not at all:
/// it is written and synced under a temporary name and renamed into place, so
/// a full disk or a crash never leaves a short key file that would block every
/// later attempt to create the store's key.
fn load_or_create_key_file(path: &Path) -> Result<Zeroizing<[u8; KEY_BYTES]>, String> {
    if let Some(existing) = read_key_file(path)? {
        return Ok(existing);
    }
    let mut kek = Zeroizing::new([0u8; KEY_BYTES]);
    getrandom::getrandom(&mut kek[..])
        .map_err(|error| format!("Geheimnisspeicher: Schlüsseldatei erzeugen: {error}"))?;
    let mut nonce = [0u8; 8];
    getrandom::getrandom(&mut nonce)
        .map_err(|error| format!("Geheimnisspeicher: Schlüsseldatei erzeugen: {error}"))?;
    let stage = path.with_extension(format!(
        "kek.tmp-{}-{}",
        std::process::id(),
        crate::creds::sealed_record::hex(&nonce)
    ));
    let written = write_stage_key_file(&stage, &kek);
    if let Err(error) = written.and_then(|()| {
        std::fs::rename(&stage, path).map_err(|error| key_file_error("anlegen", error))
    }) {
        let _ = std::fs::remove_file(&stage);
        return Err(error);
    }
    if let Some(parent) = path.parent() {
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(kek)
}

fn write_stage_key_file(stage: &Path, kek: &[u8; KEY_BYTES]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(stage)
        .map_err(|error| key_file_error("anlegen", error))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .and_then(|()| file.write_all(&kek[..]))
        .and_then(|()| file.sync_all())
        .map_err(|error| key_file_error("schreiben", error))
}

fn read_key_file(path: &Path) -> Result<Option<Zeroizing<[u8; KEY_BYTES]>>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(key_file_error("öffnen", error)),
    };
    let metadata = file
        .metadata()
        .map_err(|error| key_file_error("prüfen", error))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() != KEY_BYTES as u64
    {
        return Err(
            "Geheimnisspeicher: Schlüsseldatei muss eine eigene 32-Byte-Datei mit Rechten 0600 sein"
                .into(),
        );
    }
    let mut kek = Zeroizing::new([0u8; KEY_BYTES]);
    (&file)
        .read_exact(&mut kek[..])
        .map_err(|error| key_file_error("lesen", error))?;
    Ok(Some(kek))
}

fn key_file_error(action: &str, error: io::Error) -> String {
    format!("Geheimnisspeicher: Schlüsseldatei {action}: {error}")
}
