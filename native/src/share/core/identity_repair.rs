use super::*;

#[derive(Clone, Debug)]
pub struct IdentityRepair {
    pub identity: ShareIdentity,
    pub action: IdentityRepairAction,
    pub cleanup_warning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        _default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepairAction, String> {
        inspect_repair_need(storage).map(|need| need.action())
    }

    pub(in crate::share) fn repair_missing_with(
        default_name: String,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepair, String> {
        match inspect_repair_need(storage)? {
            IdentityRepairNeed::ReplaceIdentity(disk) => {
                Self::replace_missing_iroh(disk, default_name, storage)
            }
            IdentityRepairNeed::RotateDirectCode { disk, iroh_secret } => {
                Self::replace_missing_direct(disk, default_name, *iroh_secret, storage)
            }
        }
    }

    fn replace_missing_direct(
        disk: IdentityDisk,
        default_name: String,
        iroh_secret: iroh::SecretKey,
        storage: &mut impl IdentityPersistence,
    ) -> Result<IdentityRepair, String> {
        let (direct_lookup_id, direct_secret) =
            allocate_direct_secret(storage, Some(&disk.direct_lookup_id))?;
        let new_account = direct_secret_account(&direct_lookup_id);
        let mut identity = Self::from_disk(disk, default_name, iroh_secret, direct_secret);
        identity.direct_lookup_id = direct_lookup_id;
        if let Err(error) = identity.save_with(storage) {
            let cleanup = cleanup_secret(storage, &new_account, "neues Direkt-Secret");
            return Err(with_cleanup(error, cleanup.into_iter().collect()));
        }
        Ok(IdentityRepair {
            identity,
            action: IdentityRepairAction::DirectCodeRotated,
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
        if let Err(error) = identity.save_with(storage) {
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

    let iroh_secret = iroh::SecretKey::from_bytes(&decode_secret(&iroh_raw, "Iroh-Identitaet")?);
    validate_iroh_matches(&disk, &iroh_secret)?;
    let direct_account = direct_secret_account(&disk.direct_lookup_id);
    if load_optional_secret(storage, &direct_account, "Direkt-Code")?.is_some() {
        return Err("Share-Identitaet ist vollstaendig; Reparatur wurde verweigert".to_string());
    }
    Ok(IdentityRepairNeed::RotateDirectCode {
        disk,
        iroh_secret: Box::new(iroh_secret),
    })
}
