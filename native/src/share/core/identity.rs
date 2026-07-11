use serde::{Deserialize, Serialize};

use super::core::{
    b64, b64_decode, hex, public_fingerprint, random_bytes, random_hex_token, random_uuid_v4,
};

const IDENTITY_KEY_ACCOUNT: &str = "share:identity:iroh_secret";
const DIRECT_SECRET_PREFIX: &str = "share:identity:direct_secret:";
const SECRET_BYTES: usize = 32;

#[path = "identity_repair.rs"]
mod repair;
pub use repair::{IdentityRepair, IdentityRepairAction};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IdentityDisk {
    device_id: String,
    device_name: String,
    direct_lookup_id: String,
    public_key: String,
    fingerprint: String,
    #[serde(default)]
    node_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ShareIdentity {
    pub device_id: String,
    pub device_name: String,
    pub direct_lookup_id: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub iroh_secret: iroh::SecretKey,
    pub(crate) direct_secret: [u8; SECRET_BYTES],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCodeRotation {
    pub code: String,
    pub cleanup_warning: Option<String>,
}

impl ShareIdentity {
    pub(super) fn load_or_create_with(
        default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<Self, String> {
        let Some(disk) = load_disk(storage)? else {
            return Self::create(default_name, storage);
        };
        let iroh_bytes = load_required_secret(storage, IDENTITY_KEY_ACCOUNT, "Iroh-Identitaet")?;
        let iroh_secret = iroh::SecretKey::from_bytes(&iroh_bytes);
        validate_iroh_matches(&disk, &iroh_secret)?;
        let direct_secret = load_required_secret(
            storage,
            &direct_secret_account(&disk.direct_lookup_id),
            "Direkt-Code",
        )?;
        Ok(Self::from_disk(
            disk,
            default_name,
            iroh_secret,
            direct_secret,
        ))
    }

    fn create(device_name: String, storage: &mut impl IdentityPersistence) -> Result<Self, String> {
        let device_id =
            random_uuid_v4().map_err(|error| format!("Sichere Geraete-ID erzeugen: {error}"))?;
        let (iroh_secret, created_iroh_secret) = match storage
            .load_secret(IDENTITY_KEY_ACCOUNT)
            .map_err(|error| format!("Iroh-Identitaet lesen: {error}"))?
        {
            Some(raw) => (
                iroh::SecretKey::from_bytes(&decode_secret(&raw, "Iroh-Identitaet")?),
                false,
            ),
            None => {
                let bytes = random_bytes::<SECRET_BYTES>()
                    .map_err(|error| format!("Sicheren Iroh-Schluessel erzeugen: {error}"))?;
                let secret = iroh::SecretKey::from_bytes(&bytes);
                save_secret_verified(storage, IDENTITY_KEY_ACCOUNT, &bytes, "Iroh-Identitaet")?;
                (secret, true)
            }
        };
        let (direct_lookup_id, direct_secret) = match allocate_direct_secret(storage, None) {
            Ok(allocated) => allocated,
            Err(error) => {
                let cleanup = created_iroh_secret
                    .then(|| cleanup_secret(storage, IDENTITY_KEY_ACCOUNT, "Iroh-Secret"))
                    .flatten()
                    .into_iter()
                    .collect();
                return Err(with_cleanup(error, cleanup));
            }
        };
        let identity = Self::new(
            device_id,
            device_name,
            direct_lookup_id,
            iroh_secret,
            direct_secret,
        );
        if let Err(error) = identity.save_with(storage) {
            let mut cleanup = cleanup_secret(
                storage,
                &direct_secret_account(&identity.direct_lookup_id),
                "Direkt-Secret",
            )
            .into_iter()
            .collect::<Vec<_>>();
            if created_iroh_secret {
                cleanup.extend(cleanup_secret(storage, IDENTITY_KEY_ACCOUNT, "Iroh-Secret"));
            }
            return Err(with_cleanup(error, cleanup));
        }
        Ok(identity)
    }

    fn save_with(&self, storage: &mut impl IdentityPersistence) -> Result<(), String> {
        let contents = serde_json::to_string_pretty(&self.to_disk())
            .map_err(|error| format!("Share-Identitaet kodieren: {error}"))?;
        storage
            .save_identity(&contents)
            .map_err(|error| format!("Share-Identitaet speichern: {error}"))
    }

    pub fn direct_secret(&self) -> Vec<u8> {
        self.direct_secret.to_vec()
    }

    pub fn direct_code(&self) -> String {
        format!(
            "SE-D3-{}-{}-{}-{}",
            self.direct_lookup_id,
            hex(&self.direct_secret),
            self.fingerprint,
            self.node_id
        )
    }

    pub(super) fn regenerate_direct_code_with(
        &mut self,
        storage: &mut impl IdentityPersistence,
    ) -> Result<DirectCodeRotation, String> {
        let mut current = self.reload_persisted_with(storage)?;
        let outcome = current.rotate_loaded_direct_code_with(storage)?;
        *self = current;
        Ok(outcome)
    }

    fn rotate_loaded_direct_code_with(
        &mut self,
        storage: &mut impl IdentityPersistence,
    ) -> Result<DirectCodeRotation, String> {
        let old_account = direct_secret_account(&self.direct_lookup_id);
        let (direct_lookup_id, direct_secret) =
            allocate_direct_secret(storage, Some(&self.direct_lookup_id))?;
        let new_account = direct_secret_account(&direct_lookup_id);
        let mut candidate = self.clone();
        candidate.direct_lookup_id = direct_lookup_id;
        candidate.direct_secret = direct_secret;
        if let Err(error) = candidate.save_with(storage) {
            let cleanup = storage
                .delete_secret(&new_account)
                .err()
                .map(|cleanup_error| format!("neues Direkt-Secret: {cleanup_error}"))
                .into_iter()
                .collect();
            return Err(with_cleanup(error, cleanup));
        }
        *self = candidate;
        let cleanup_warning = storage
            .delete_secret(&old_account)
            .err()
            .map(|error| format!("Alter Direkt-Code konnte nicht entfernt werden: {error}"));
        Ok(DirectCodeRotation {
            code: self.direct_code(),
            cleanup_warning,
        })
    }

    pub(super) fn set_device_name_with(
        &mut self,
        name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<(), String> {
        let mut current = self.reload_persisted_with(storage)?;
        current.rename_loaded_with(name, storage)?;
        *self = current;
        Ok(())
    }

    fn rename_loaded_with(
        &mut self,
        name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed == self.device_name {
            return Ok(());
        }
        let mut candidate = self.clone();
        candidate.device_name = trimmed.to_string();
        candidate.save_with(storage)?;
        *self = candidate;
        Ok(())
    }

    fn reload_persisted_with(
        &self,
        storage: &mut impl IdentityPersistence,
    ) -> Result<Self, String> {
        let disk = load_disk(storage)?
            .ok_or_else(|| "Share-Identitaet fehlt; Aenderung wurde verweigert".to_string())?;
        let iroh_bytes = load_required_secret(storage, IDENTITY_KEY_ACCOUNT, "Iroh-Identitaet")?;
        let iroh_secret = iroh::SecretKey::from_bytes(&iroh_bytes);
        validate_iroh_matches(&disk, &iroh_secret)?;
        let direct_secret = load_required_secret(
            storage,
            &direct_secret_account(&disk.direct_lookup_id),
            "Direkt-Code",
        )?;
        Ok(Self::from_disk(
            disk,
            self.device_name.clone(),
            iroh_secret,
            direct_secret,
        ))
    }

    fn to_disk(&self) -> IdentityDisk {
        IdentityDisk {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            direct_lookup_id: self.direct_lookup_id.clone(),
            public_key: self.public_key.clone(),
            fingerprint: self.fingerprint.clone(),
            node_id: Some(self.node_id.clone()),
        }
    }

    fn from_disk(
        disk: IdentityDisk,
        default_name: String,
        iroh_secret: iroh::SecretKey,
        direct_secret: [u8; SECRET_BYTES],
    ) -> Self {
        let device_name = if disk.device_name.trim().is_empty() {
            default_name
        } else {
            disk.device_name
        };
        Self::new(
            disk.device_id,
            device_name,
            disk.direct_lookup_id,
            iroh_secret,
            direct_secret,
        )
    }

    fn new(
        device_id: String,
        device_name: String,
        direct_lookup_id: String,
        iroh_secret: iroh::SecretKey,
        direct_secret: [u8; SECRET_BYTES],
    ) -> Self {
        let node_id = iroh_secret.public().to_string();
        let public_key = node_id.clone();
        Self {
            device_id,
            device_name,
            direct_lookup_id,
            fingerprint: public_fingerprint(public_key.as_bytes()),
            public_key,
            node_id,
            iroh_secret,
            direct_secret,
        }
    }
}

pub(crate) fn direct_secret_account(lookup_id: &str) -> String {
    format!("{DIRECT_SECRET_PREFIX}{lookup_id}")
}

fn allocate_direct_secret(
    storage: &mut impl IdentityPersistence,
    excluded_lookup_id: Option<&str>,
) -> Result<(String, [u8; SECRET_BYTES]), String> {
    for _ in 0..16 {
        let lookup_id = random_hex_token::<12>()
            .map_err(|error| format!("Sichere Direkt-ID erzeugen: {error}"))?;
        if excluded_lookup_id == Some(lookup_id.as_str()) {
            continue;
        }
        let account = direct_secret_account(&lookup_id);
        if storage
            .load_secret(&account)
            .map_err(|error| format!("Direkt-Secret pruefen: {error}"))?
            .is_some()
        {
            continue;
        }
        let secret = random_bytes::<SECRET_BYTES>()
            .map_err(|error| format!("Sicheres Direkt-Secret erzeugen: {error}"))?;
        save_secret_verified(storage, &account, &secret, "Direkt-Secret")?;
        return Ok((lookup_id, secret));
    }
    Err("Kein freier Speicherplatz fuer einen neuen Direkt-Code gefunden".into())
}

fn save_secret_verified(
    storage: &mut impl IdentityPersistence,
    account: &str,
    secret: &[u8; SECRET_BYTES],
    label: &str,
) -> Result<(), String> {
    let result = storage
        .save_secret(account, &b64(secret))
        .map_err(|error| format!("{label} speichern: {error}"))
        .and_then(|()| match load_optional_secret(storage, account, label)? {
            Some(stored) if stored == *secret => Ok(()),
            Some(_) => Err(format!("{label} wurde mit anderen Bytes gespeichert")),
            None => Err(format!("{label} wurde nicht im sicheren Speicher behalten")),
        });
    if let Err(error) = result {
        let cleanup = cleanup_secret(storage, account, label)
            .into_iter()
            .collect();
        return Err(with_cleanup(error, cleanup));
    }
    Ok(())
}

fn cleanup_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Option<String> {
    storage
        .delete_secret(account)
        .err()
        .map(|error| format!("{label}: {error}"))
}

fn load_required_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Result<[u8; SECRET_BYTES], String> {
    load_optional_secret(storage, account, label)?
        .ok_or_else(|| format!("{label} fehlt im sicheren Speicher"))
}

fn load_optional_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Result<Option<[u8; SECRET_BYTES]>, String> {
    storage
        .load_secret(account)
        .map_err(|error| format!("{label} lesen: {error}"))?
        .map(|raw| decode_secret(&raw, label))
        .transpose()
}

fn decode_secret(raw: &str, label: &str) -> Result<[u8; SECRET_BYTES], String> {
    b64_decode(raw)
        .map_err(|error| format!("{label} ist ungueltig: {error}"))?
        .try_into()
        .map_err(|_| format!("{label} hat nicht {SECRET_BYTES} Bytes"))
}

fn validate_disk(disk: &IdentityDisk) -> Result<(), String> {
    if disk.device_id.trim().is_empty() || disk.direct_lookup_id.trim().is_empty() {
        return Err("Share-Identitaet enthaelt keine stabile Geraete- oder Direkt-ID".into());
    }
    Ok(())
}

fn validate_iroh_matches(disk: &IdentityDisk, iroh_secret: &iroh::SecretKey) -> Result<(), String> {
    let node_id = iroh_secret.public().to_string();
    if disk
        .node_id
        .as_deref()
        .filter(|saved| !saved.trim().is_empty())
        .is_some_and(|saved| saved != node_id)
    {
        return Err("Share-Identitaet passt nicht zum sicher gespeicherten Iroh-Schluessel".into());
    }
    Ok(())
}

fn load_disk(storage: &mut impl IdentityPersistence) -> Result<Option<IdentityDisk>, String> {
    storage
        .load_identity()
        .map_err(|error| format!("Share-Identitaet lesen: {error}"))?
        .map(|raw| {
            let disk: IdentityDisk = serde_json::from_str(&raw)
                .map_err(|error| format!("Share-Identitaet ist beschaedigt: {error}"))?;
            validate_disk(&disk)?;
            Ok(disk)
        })
        .transpose()
}

fn with_cleanup(error: String, cleanup: Vec<String>) -> String {
    if cleanup.is_empty() {
        error
    } else {
        format!(
            "{error}; Bereinigung fehlgeschlagen: {}",
            cleanup.join(", ")
        )
    }
}

pub(super) trait IdentityPersistence {
    fn load_identity(&mut self) -> Result<Option<String>, String>;
    fn save_identity(&mut self, contents: &str) -> Result<(), String>;
    fn load_secret(&mut self, account: &str) -> Result<Option<String>, String>;
    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String>;
    fn delete_secret(&mut self, account: &str) -> Result<(), String>;
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
