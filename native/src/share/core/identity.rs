use serde::{Deserialize, Serialize};

use super::core::{
    b64, b64_decode, hex, public_fingerprint, random_bytes, random_hex_token, random_uuid_v4,
};

const IDENTITY_KEY_ACCOUNT: &str = "share:identity:iroh_secret";
const DIRECT_SECRET_PREFIX: &str = "share:identity:direct_secret:";
const SECRET_BYTES: usize = 32;

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
        let Some(raw) = storage
            .load_identity()
            .map_err(|error| format!("Share-Identitaet lesen: {error}"))?
        else {
            return Self::create(default_name, storage);
        };
        let disk: IdentityDisk = serde_json::from_str(&raw)
            .map_err(|error| format!("Share-Identitaet ist beschaedigt: {error}"))?;
        validate_disk(&disk)?;
        let iroh_bytes = load_required_secret(storage, IDENTITY_KEY_ACCOUNT, "Iroh-Identitaet")?;
        let iroh_secret = iroh::SecretKey::from_bytes(&iroh_bytes);
        let direct_secret = load_required_secret(
            storage,
            &direct_secret_account(&disk.direct_lookup_id),
            "Direkt-Code",
        )?;
        let node_id = iroh_secret.public().to_string();
        if disk
            .node_id
            .as_deref()
            .filter(|saved| !saved.trim().is_empty())
            .is_some_and(|saved| saved != node_id)
        {
            return Err(
                "Share-Identitaet passt nicht zum sicher gespeicherten Iroh-Schluessel".into(),
            );
        }
        let public_key = node_id.clone();
        let fingerprint = public_fingerprint(public_key.as_bytes());
        Ok(Self {
            device_id: disk.device_id,
            device_name: if disk.device_name.trim().is_empty() {
                default_name
            } else {
                disk.device_name
            },
            direct_lookup_id: disk.direct_lookup_id,
            public_key,
            fingerprint,
            node_id,
            iroh_secret,
            direct_secret,
        })
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
                storage
                    .save_secret(IDENTITY_KEY_ACCOUNT, &b64(&secret.to_bytes()))
                    .map_err(|error| format!("Iroh-Identitaet speichern: {error}"))?;
                (secret, true)
            }
        };
        let (direct_lookup_id, direct_secret) = match allocate_direct_secret(storage) {
            Ok(allocated) => allocated,
            Err(error) => {
                if created_iroh_secret {
                    let _ = storage.delete_secret(IDENTITY_KEY_ACCOUNT);
                }
                return Err(error);
            }
        };
        let node_id = iroh_secret.public().to_string();
        let public_key = node_id.clone();
        let identity = Self {
            device_id,
            device_name,
            direct_lookup_id,
            fingerprint: public_fingerprint(public_key.as_bytes()),
            public_key,
            node_id,
            iroh_secret,
            direct_secret,
        };
        if let Err(error) = identity.save_with(storage) {
            let mut cleanup = Vec::new();
            if let Err(cleanup_error) =
                storage.delete_secret(&direct_secret_account(&identity.direct_lookup_id))
            {
                cleanup.push(format!("Direkt-Secret: {cleanup_error}"));
            }
            if created_iroh_secret {
                if let Err(cleanup_error) = storage.delete_secret(IDENTITY_KEY_ACCOUNT) {
                    cleanup.push(format!("Iroh-Secret: {cleanup_error}"));
                }
            }
            return Err(with_cleanup(error, cleanup));
        }
        Ok(identity)
    }

    pub(super) fn save_with(&self, storage: &mut impl IdentityPersistence) -> Result<(), String> {
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
        let old_account = direct_secret_account(&self.direct_lookup_id);
        let (direct_lookup_id, direct_secret) = allocate_direct_secret(storage)?;
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
}

pub(crate) fn direct_secret_account(lookup_id: &str) -> String {
    format!("{DIRECT_SECRET_PREFIX}{lookup_id}")
}

fn allocate_direct_secret(
    storage: &mut impl IdentityPersistence,
) -> Result<(String, [u8; SECRET_BYTES]), String> {
    for _ in 0..16 {
        let lookup_id = random_hex_token::<12>()
            .map_err(|error| format!("Sichere Direkt-ID erzeugen: {error}"))?;
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
        storage
            .save_secret(&account, &b64(&secret))
            .map_err(|error| format!("Direkt-Secret speichern: {error}"))?;
        return Ok((lookup_id, secret));
    }
    Err("Kein freier Speicherplatz fuer einen neuen Direkt-Code gefunden".into())
}

fn load_required_secret(
    storage: &mut impl IdentityPersistence,
    account: &str,
    label: &str,
) -> Result<[u8; SECRET_BYTES], String> {
    let raw = storage
        .load_secret(account)
        .map_err(|error| format!("{label} lesen: {error}"))?
        .ok_or_else(|| format!("{label} fehlt im sicheren Speicher"))?;
    decode_secret(&raw, label)
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
mod tests {
    use std::collections::HashMap;

    use super::{direct_secret_account, IdentityPersistence, ShareIdentity};

    #[derive(Default)]
    struct FakePersistence {
        identity: Option<String>,
        secrets: HashMap<String, String>,
        fail_identity_save: bool,
        fail_secret_save: bool,
    }

    impl IdentityPersistence for FakePersistence {
        fn load_identity(&mut self) -> Result<Option<String>, String> {
            Ok(self.identity.clone())
        }

        fn save_identity(&mut self, contents: &str) -> Result<(), String> {
            if self.fail_identity_save {
                Err("disk full".into())
            } else {
                self.identity = Some(contents.to_string());
                Ok(())
            }
        }

        fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
            Ok(self.secrets.get(account).cloned())
        }

        fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
            if self.fail_secret_save {
                Err("secure store unavailable".into())
            } else {
                self.secrets.insert(account.to_string(), secret.to_string());
                Ok(())
            }
        }

        fn delete_secret(&mut self, account: &str) -> Result<(), String> {
            self.secrets.remove(account);
            Ok(())
        }
    }

    #[test]
    fn missing_persisted_direct_secret_never_produces_a_code() {
        let mut storage = FakePersistence::default();
        let identity = ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        storage
            .secrets
            .remove(&direct_secret_account(&identity.direct_lookup_id));
        assert!(ShareIdentity::load_or_create_with("device".into(), &mut storage).is_err());
    }

    #[test]
    fn failed_initial_metadata_write_cleans_up_new_secrets() {
        let mut storage = FakePersistence {
            fail_identity_save: true,
            ..FakePersistence::default()
        };
        assert!(ShareIdentity::load_or_create_with("device".into(), &mut storage).is_err());
        assert!(storage.secrets.is_empty());
        assert!(storage.identity.is_none());
    }

    #[test]
    fn failed_rotation_secret_write_keeps_the_old_identity_and_code() {
        let mut storage = FakePersistence::default();
        let mut identity =
            ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        let old_code = identity.direct_code();
        let old_lookup = identity.direct_lookup_id.clone();
        storage.fail_secret_save = true;
        assert!(identity.regenerate_direct_code_with(&mut storage).is_err());
        assert_eq!(identity.direct_code(), old_code);
        assert_eq!(identity.direct_lookup_id, old_lookup);
    }

    #[test]
    fn failed_rotation_metadata_write_keeps_old_code_and_removes_candidate_secret() {
        let mut storage = FakePersistence::default();
        let mut identity =
            ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        let old_code = identity.direct_code();
        let old_accounts = storage.secrets.clone();
        storage.fail_identity_save = true;
        assert!(identity.regenerate_direct_code_with(&mut storage).is_err());
        assert_eq!(identity.direct_code(), old_code);
        assert_eq!(storage.secrets, old_accounts);
    }

    #[test]
    fn successful_rotation_returns_the_single_persisted_code() {
        let mut storage = FakePersistence::default();
        let mut identity =
            ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        let old_account = direct_secret_account(&identity.direct_lookup_id);
        let outcome = identity.regenerate_direct_code_with(&mut storage).unwrap();
        assert_eq!(outcome.code, identity.direct_code());
        assert_eq!(outcome.cleanup_warning, None);
        assert!(!storage.secrets.contains_key(&old_account));
        assert!(storage
            .secrets
            .contains_key(&direct_secret_account(&identity.direct_lookup_id)));
    }
}
