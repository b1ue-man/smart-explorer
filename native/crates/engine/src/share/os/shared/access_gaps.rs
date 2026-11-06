//! R1/R9 (plan 2026-11-06): which paired devices cannot open this device's
//! exports, and why, projected from `ShareProfiles` alone (every surface has
//! them: desktop snapshot, CLI profile file, Android bridge). Each gap names
//! the action that allows the device; nothing here grants by itself.
use crate::share::{DirectAccessState, DirectGrantState, RelationChange, ShareProfiles};

/// Why one paired device cannot open this device's exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessGapKind {
    /// This device reaches the other one, but never opened its exports to
    /// it (paired „nur in eine Richtung“ or before R1).
    NotShared,
    /// The grant waits for re-confirmation after this device's code
    /// rotation or identity repair.
    Reconfirm,
    /// The grant exists, but its rights wait for „Rechte festlegen“.
    RightsPending,
}

impl AccessGapKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotShared => "not_shared",
            Self::Reconfirm => "reconfirm",
            Self::RightsPending => "rights_pending",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessGap {
    pub kind: AccessGapKind,
    pub device_id: String,
    pub device_name: String,
    /// This device's contact of the other device, when it has one.
    pub contact_id: Option<String>,
    /// One sentence: the device and what it cannot do.
    pub message: String,
    /// The action that allows it, as the surfaces name it.
    pub action: String,
}

/// Every gap, Reconfirm first (they used to work), then pending rights,
/// then one-way pairings.
pub fn access_gaps(profiles: &ShareProfiles) -> Vec<AccessGap> {
    let mut gaps = Vec::new();
    for grant in &profiles.direct_grants {
        let contact = profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.remote_device_id.as_deref() == Some(grant.device_id.as_str()));
        let name = shown_name(&grant.device_name, contact.map(|c| c.display_name.as_str()));
        let (kind, message, action) = match grant.state {
            DirectGrantState::Reconfirm => (
                AccessGapKind::Reconfirm,
                format!("{name} kann deine Freigaben erst nach erneuter Bestätigung öffnen (Direkt-Code erneuert oder Identität repariert)"),
                "Neu bestätigen".to_string(),
            ),
            DirectGrantState::Accepted
                if profiles
                    .pending_rights
                    .iter()
                    .any(|pending| pending.target.is_direct(&grant.device_id)) =>
            {
                (
                    AccessGapKind::RightsPending,
                    format!("{name} ist verbunden, hat aber noch keine Rechte (Rechte offen): es sieht keine Ordner"),
                    "Rechte festlegen".to_string(),
                )
            }
            _ => continue,
        };
        gaps.push(AccessGap {
            kind,
            device_id: grant.device_id.clone(),
            device_name: name,
            contact_id: contact.map(|contact| contact.id.clone()),
            message,
            action,
        });
    }
    for contact in &profiles.direct_contacts {
        let Some(device_id) = contact.remote_device_id.as_deref() else {
            continue;
        };
        if contact.access_state != DirectAccessState::Accepted
            || profiles
                .direct_grants
                .iter()
                .any(|grant| grant.device_id == device_id)
        {
            continue;
        }
        let name = shown_name(&contact.display_name, None);
        gaps.push(AccessGap {
            kind: AccessGapKind::NotShared,
            device_id: device_id.to_string(),
            device_name: name.clone(),
            contact_id: Some(contact.id.clone()),
            message: format!("Du erreichst {name}, aber {name} kann deine Freigaben nicht öffnen (nur in eine Richtung verbunden)"),
            action: "Eigene Freigaben für dieses Gerät öffnen".to_string(),
        });
    }
    gaps.sort_by_key(|gap| match gap.kind {
        AccessGapKind::Reconfirm => 0,
        AccessGapKind::RightsPending => 1,
        AccessGapKind::NotShared => 2,
    });
    gaps
}

fn shown_name(name: &str, fallback: Option<&str>) -> String {
    [Some(name), fallback]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .unwrap_or("Unbenanntes Gerät")
        .to_string()
}

/// R9: „Alle neu bestätigen“: every grant suspended for re-confirmation is
/// active again in one transaction, exactly as one „Bestätigen“ each (Exec
/// stays off, a grant without rights waits for them). Blocked grants stay
/// blocked.
pub fn reconfirm_all_direct_grants(default_home: Option<String>) -> Result<RelationChange, String> {
    let now = crate::share::core_now_secs();
    let mut changed = false;
    let profiles = ShareProfiles::mutate_persisted(default_home, |profiles| {
        let devices: Vec<String> = profiles
            .direct_grants
            .iter()
            .filter(|grant| grant.state == DirectGrantState::Reconfirm)
            .map(|grant| grant.device_id.clone())
            .collect();
        for device_id in devices {
            changed |= profiles.allow_direct_grant_again(&device_id, now)?;
        }
        Ok(())
    })?;
    Ok(RelationChange { profiles, changed })
}

#[cfg(test)]
#[path = "access_gaps_tests.rs"]
mod tests;
