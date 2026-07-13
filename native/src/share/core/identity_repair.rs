use super::*;

#[derive(Clone, Debug)]
pub struct IdentityRepair {
    pub identity: ShareIdentity,
    pub action: IdentityRepairAction,
    pub cleanup_warning: Option<String>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityRepairAction {
    IdentityReplaced,
    DirectCodeRotated,
}

enum IdentityRepairNeed {
    ReplaceIdentity(IdentityDisk),
    RotateDirectCode {
        disk: IdentityDisk,
        iroh_secret: Box<iroh::SecretKey>,
    },
}

impl IdentityRepairNeed {
    fn action(&self) -> IdentityRepairAction {
        match self {
            Self::ReplaceIdentity(_) => IdentityRepairAction::IdentityReplaced,
            Self::RotateDirectCode { .. } => IdentityRepairAction::DirectCodeRotated,
        }
    }
}

impl ShareIdentity {
    pub(in crate::share) fn repair_action_needed_with(
        default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepairAction, String> {
        let pending = Self::pending_cleanup_action_with(storage)?;
        if let Some(action) = pending {
            if Self::load_or_create_with(default_name, storage).is_ok() {
                return Ok(action);
            }
        }
        inspect_repair_need(storage).map(|need| strongest_cleanup_action(pending, need.action()))
    }

    pub(in crate::share) fn repair_missing_with(
        default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepair, String> {
        let pending = Self::pending_cleanup_action_with(storage)?;
        if let Some(action) = pending {
            if let Ok(identity) = Self::load_or_create_with(default_name.clone(), storage) {
                return Ok(IdentityRepair {
                    identity,
                    action,
                    cleanup_warning: None,
                });
            }
        }
        match inspect_repair_need(storage)? {
            IdentityRepairNeed::ReplaceIdentity(disk) => {
                Self::replace_missing_iroh(disk, default_name, storage)
            }
            IdentityRepairNeed::RotateDirectCode { disk, iroh_secret } => {
                let action =
                    strongest_cleanup_action(pending, IdentityRepairAction::DirectCodeRotated);
                Self::replace_missing_direct(disk, default_name, *iroh_secret, action, storage)
            }
        }
    }

    fn replace_missing_direct(
        disk: IdentityDisk,
        default_name: String,
        iroh_secret: iroh::SecretKey,
        cleanup_action: IdentityRepairAction,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepair, String> {
        let (direct_lookup_id, direct_secret) =
            allocate_direct_secret(storage, Some(&disk.direct_lookup_id))?;
        let new_account = direct_secret_account(&direct_lookup_id);
        let mut identity = Self::from_disk(disk, default_name, iroh_secret, direct_secret);
        identity.direct_lookup_id = direct_lookup_id;
        if let Err(error) = identity.save_with_pending_cleanup(storage, cleanup_action) {
            let cleanup = cleanup_secret(storage, &new_account, "neues Direkt-Secret");
            return Err(with_cleanup(error, cleanup.into_iter().collect()));
        }
        Ok(IdentityRepair {
            identity,
            action: cleanup_action,
            cleanup_warning: None,
        })
    }

    fn replace_missing_iroh(
        disk: IdentityDisk,
        default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepair, String> {
        let device_id =
            random_uuid_v4().map_err(|error| format!("Sichere Geraete-ID erzeugen: {error}"))?;
        let old_direct_account = direct_secret_account(&disk.direct_lookup_id);
        let device_name = if disk.device_name.trim().is_empty() {
            default_name
        } else {
            disk.device_name
        };
        let iroh_bytes = random_bytes::<SECRET_BYTES>()
            .map_err(|error| format!("Sicheren Iroh-Schluessel erzeugen: {error}"))?;
        save_secret_verified(
            storage,
            IDENTITY_KEY_ACCOUNT,
            &iroh_bytes,
            "Iroh-Identitaet",
        )?;
        let iroh_secret = iroh::SecretKey::from_bytes(&iroh_bytes);
        let (direct_lookup_id, direct_secret) =
            match allocate_direct_secret(storage, Some(&disk.direct_lookup_id)) {
                Ok(allocated) => allocated,
                Err(error) => {
                    let cleanup =
                        cleanup_secret(storage, IDENTITY_KEY_ACCOUNT, "neue Iroh-Identitaet");
                    return Err(with_cleanup(error, cleanup.into_iter().collect()));
                }
            };
        let new_direct_account = direct_secret_account(&direct_lookup_id);
        let identity = Self::new(
            device_id,
            device_name,
            direct_lookup_id,
            iroh_secret,
            direct_secret,
        );
        if let Err(error) =
            identity.save_with_pending_cleanup(storage, IdentityRepairAction::IdentityReplaced)
        {
            let cleanup = [
                cleanup_secret(storage, &new_direct_account, "neues Direkt-Secret"),
                cleanup_secret(storage, IDENTITY_KEY_ACCOUNT, "neue Iroh-Identitaet"),
            ]
            .into_iter()
            .flatten()
            .collect();
            return Err(with_cleanup(error, cleanup));
        }
        let cleanup_warning = storage
            .delete_secret(&old_direct_account)
            .err()
            .map(|error| format!("Alter Direkt-Code konnte nicht entfernt werden: {error}"));
        Ok(IdentityRepair {
            identity,
            action: IdentityRepairAction::IdentityReplaced,
            cleanup_warning,
        })
    }
}

fn strongest_cleanup_action(
    pending: Option<IdentityRepairAction>,
    required: IdentityRepairAction,
) -> IdentityRepairAction {
    if pending == Some(IdentityRepairAction::IdentityReplaced)
        || required == IdentityRepairAction::IdentityReplaced
    {
        IdentityRepairAction::IdentityReplaced
    } else {
        IdentityRepairAction::DirectCodeRotated
    }
}

fn inspect_repair_need(
    storage: &mut impl IdentityPersistence,
) -> Result<IdentityRepairNeed, String> {
    let disk = load_disk(storage)?.ok_or_else(|| {
        "Share-Identitaet fehlt; es gibt keine unvollstaendige Identitaet zu reparieren".to_string()
    })?;
    let iroh_raw = storage
        .load_secret(IDENTITY_KEY_ACCOUNT)
        .map_err(|error| format!("Iroh-Identitaet lesen: {error}"))?;
    let Some(iroh_raw) = iroh_raw else {
        return Ok(IdentityRepairNeed::ReplaceIdentity(disk));
    };

    let iroh_bytes = match decode_secret(&iroh_raw, "Iroh-Identitaet") {
        Ok(secret) => secret,
        // An interrupted full replacement can leave the old metadata next to
        // a partially written replacement secret. Explicit repair must be
        // able to replace that unusable generation instead of requiring
        // manual credential-store surgery.
        Err(_) => return Ok(IdentityRepairNeed::ReplaceIdentity(disk)),
    };
    let iroh_secret = iroh::SecretKey::from_bytes(&iroh_bytes);
    if validate_iroh_matches(&disk, &iroh_secret).is_err() {
        return Ok(IdentityRepairNeed::ReplaceIdentity(disk));
    }
    let direct_account = direct_secret_account(&disk.direct_lookup_id);
    if load_optional_secret(storage, &direct_account, "Direkt-Code")?.is_some() {
        return Err("Share-Identitaet ist vollstaendig; Reparatur wurde verweigert".to_string());
    }
    Ok(IdentityRepairNeed::RotateDirectCode {
        disk,
        iroh_secret: Box::new(iroh_secret),
    })
}

#[cfg(test)]
mod crash_tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn full_replacement_marker_survives_a_later_missing_direct_secret() {
        let mut storage = MemoryPersistence::default();
        ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        storage.secrets.remove(IDENTITY_KEY_ACCOUNT);
        let replaced = ShareIdentity::repair_missing_with("device".into(), &mut storage).unwrap();
        let replaced_device_id = replaced.identity.device_id.clone();
        storage
            .secrets
            .remove(&direct_secret_account(&replaced.identity.direct_lookup_id));

        let repaired = ShareIdentity::repair_missing_with("device".into(), &mut storage).unwrap();

        assert_eq!(repaired.action, IdentityRepairAction::IdentityReplaced);
        assert_eq!(repaired.identity.device_id, replaced_device_id);
        assert_eq!(
            ShareIdentity::pending_cleanup_action_with(&mut storage).unwrap(),
            Some(IdentityRepairAction::IdentityReplaced)
        );
    }

    #[test]
    fn legacy_metadata_without_node_id_still_binds_the_fixed_iroh_secret() {
        let mut storage = MemoryPersistence::default();
        ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap();
        let mut disk: serde_json::Value =
            serde_json::from_str(storage.identity.as_deref().unwrap()).unwrap();
        disk.as_object_mut().unwrap().remove("node_id");
        storage.identity = Some(serde_json::to_string(&disk).unwrap());
        let replacement = iroh::SecretKey::from_bytes(&[0xa7; 32]);
        storage
            .secrets
            .insert(IDENTITY_KEY_ACCOUNT.into(), b64(&replacement.to_bytes()));

        let error = ShareIdentity::load_or_create_with("device".into(), &mut storage).unwrap_err();

        assert!(error.contains("passt nicht"));
        assert_eq!(
            ShareIdentity::repair_action_needed_with("device".into(), &mut storage).unwrap(),
            IdentityRepairAction::IdentityReplaced
        );
    }

    #[derive(Default)]
    struct MemoryPersistence {
        identity: Option<String>,
        secrets: HashMap<String, String>,
    }

    impl IdentityPersistence for MemoryPersistence {
        fn load_identity(&mut self) -> Result<Option<String>, String> {
            Ok(self.identity.clone())
        }

        fn save_identity(&mut self, contents: &str) -> Result<(), String> {
            self.identity = Some(contents.to_string());
            Ok(())
        }

        fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
            Ok(self.secrets.get(account).cloned())
        }

        fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
            self.secrets.insert(account.to_string(), secret.to_string());
            Ok(())
        }

        fn delete_secret(&mut self, account: &str) -> Result<(), String> {
            self.secrets.remove(account);
            Ok(())
        }
    }
}
