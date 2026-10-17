//! Creating a vault (VLT-M5, C1.6, C7.1 T4): format 8, `SIV_GCM`, the
//! shortening threshold of the user (36–220, default 220), a new password of
//! at least 8 characters. Files in the order of Cryptomator: key file,
//! configuration, root content folder with its `dirid.c9r`; a failure
//! removes what the call created. Child module of `manage.rs`.
use std::io;

use crate::cryptomator::{
    content_dir, encrypt_bytes, CipherCombo, ContentCryptor, Masterkey, MasterkeyCell,
    MasterkeyFile, VaultConfig, DIR_ID_BACKUP_FILE, MASTERKEY_FILE, ROOT_DIR_ID, VAULT_CONFIG_FILE,
};
use crate::vault::core::path_map::storage_path;
use crate::vault::core::registry_types::SiteRecord;
use crate::vfs::Backend;

use super::super::open::ResolveLocation;
use super::super::password::{check_new_password, write_new};
use super::super::registry_store::RegistryStore;
use super::{enter, new_label, refused, storage, ManageError};

/// Undoes a creation that failed half way (only what this call made).
#[derive(Default)]
struct Made {
    files: Vec<String>,
    folders: Vec<String>,
}

impl Made {
    fn undo(&self, inner: &dyn Backend) {
        for file in self.files.iter().rev() {
            let _ = inner.remove_file(file);
        }
        for folder in self.folders.iter().rev() {
            let _ = inner.remove_dir(folder);
        }
    }
}

/// The vault root: created when missing, else it must be an empty folder.
fn prepare_root(inner: &dyn Backend, root: &str, made: &mut Made) -> Result<(), ManageError> {
    match inner.stat(root) {
        Ok(meta) if meta.is_dir => {
            if inner.list_dir(root).map_err(storage)?.is_empty() {
                Ok(())
            } else {
                Err(ManageError::Refused(
                    "Der Ordner ist nicht leer: für einen neuen Tresor einen leeren oder neuen \
                     Ordner wählen"
                        .into(),
                ))
            }
        }
        Ok(_) => Err(ManageError::Refused(
            "Am Speicherort liegt eine Datei".into(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            inner.create_dir_new(root).map_err(storage)?;
            made.folders.push(root.to_string());
            Ok(())
        }
        Err(error) => Err(storage(error)),
    }
}

fn create_in(
    inner: &dyn Backend,
    root: &str,
    key_file: &[u8],
    token: &str,
    root_content: &str,
    dir_id_backup: &[u8],
    made: &mut Made,
) -> Result<(), ManageError> {
    prepare_root(inner, root, made)?;
    // The order of Cryptomator: key file, configuration, root folder.
    for (name, bytes) in [
        (MASTERKEY_FILE, key_file),
        (VAULT_CONFIG_FILE, token.as_bytes()),
    ] {
        let path = storage_path(root, name);
        write_new(inner, &path, bytes).map_err(storage)?;
        made.files.push(path);
    }
    let mut folder = String::new();
    for part in root_content.split('/') {
        folder = if folder.is_empty() {
            part.to_string()
        } else {
            format!("{folder}/{part}")
        };
        let path = storage_path(root, &folder);
        inner.create_dir_new(&path).map_err(storage)?;
        made.folders.push(path);
    }
    let backup = storage_path(root, &format!("{root_content}/{DIR_ID_BACKUP_FILE}"));
    write_new(inner, &backup, dir_id_backup).map_err(storage)?;
    made.files.push(backup);
    Ok(())
}

/// Creates a new vault (format 8, `SIV_GCM`, C1.6) in the folder `root`:
/// `masterkey.cryptomator`, `vault.cryptomator`, the root content folder
/// and its `dirid.c9r`. A failure removes what this call created.
pub(crate) fn create_vault(
    inner: &dyn Backend,
    root: &str,
    password: &str,
    shortening_threshold: u32,
    available: Option<u64>,
) -> Result<VaultConfig, ManageError> {
    check_new_password(password).map_err(ManageError::Invalid)?;
    let config = VaultConfig::create(shortening_threshold)
        .map_err(|error| ManageError::Invalid(error.to_string()))?;
    let key = Masterkey::generate().map_err(refused)?;
    let key_file = MasterkeyFile::create(&key, password, available).map_err(refused)?;
    let token = config.sign(&key).map_err(refused)?;
    let root_content = content_dir(&key, ROOT_DIR_ID).map_err(refused)?;
    // The root's own ID (empty) as `dirid.c9r`: only a header (C1.7).
    let cryptor = ContentCryptor::new(
        MasterkeyCell::new(Masterkey::from_raw(&key.raw())),
        CipherCombo::SivGcm,
    );
    let dir_id_backup = encrypt_bytes(&cryptor, ROOT_DIR_ID.as_bytes()).map_err(refused)?;
    drop((key, cryptor));
    let mut made = Made::default();
    let created = create_in(
        inner,
        root,
        &key_file,
        &token,
        &root_content,
        &dir_id_backup,
        &mut made,
    );
    if let Err(error) = created {
        made.undo(inner);
        return Err(error);
    }
    Ok(config)
}

/// Creates a vault at `location` and enters it (T4).
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_site(
    store: &RegistryStore,
    location: &str,
    label: Option<&str>,
    password: &str,
    shortening_threshold: u32,
    available: Option<u64>,
    resolve: ResolveLocation<'_>,
) -> Result<SiteRecord, ManageError> {
    check_new_password(password).map_err(ManageError::Invalid)?;
    let location = location.trim();
    let label = new_label(store, label, location)?;
    let (inner, root) = resolve(location).map_err(storage)?;
    let config = create_vault(
        inner.as_ref(),
        &root,
        password,
        shortening_threshold,
        available,
    )?;
    enter(store, location, label, &config.jti).map_err(|error| {
        ManageError::Registry(format!(
            "Tresor angelegt, aber nicht eingetragen ({error}); mit „se vault add“ eintragen"
        ))
    })
}
