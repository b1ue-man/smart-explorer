pub(super) fn upsert_mount(
    mounts: &mut Vec<crate::mount::MountSnapshot>,
    snapshot: crate::mount::MountSnapshot,
) {
    if matches!(&snapshot.status, crate::mount::MountStatus::Unmounted) {
        mounts.retain(|mount| mount.config.id != snapshot.config.id);
        return;
    }
    if let Some(existing) = mounts
        .iter_mut()
        .find(|mount| mount.config.id == snapshot.config.id)
    {
        *existing = snapshot;
    } else {
        mounts.push(snapshot);
    }
}

pub(super) fn mount_status_alert(
    previous: Option<&crate::mount::MountStatus>,
    mount: &crate::mount::MountSnapshot,
) -> Option<String> {
    if previous.is_some_and(|status| status == &mount.status) {
        return None;
    }
    let label = &mount.config.label;
    match &mount.status {
        crate::mount::MountStatus::Conflict { path, detail, .. } => Some(format!(
            "Laufwerk \"{label}\": Die Wiederherstellung der Aenderung an {path} ist noch offen: {detail}. Vorhandene Recovery-Daten und das Journal bleiben erhalten; Details stehen im Laufwerksmanager."
        )),
        crate::mount::MountStatus::Failed { detail }
            if mount.recovery == crate::mount::MountRecovery::Required => Some(format!(
            "Laufwerk \"{label}\" ist ausgefallen: {detail}. Vorhandene lokale Aenderungen und das Recovery-Journal bleiben erhalten; Details stehen im Laufwerksmanager."
        )),
        crate::mount::MountStatus::Failed { detail }
            if mount.recovery == crate::mount::MountRecovery::Unknown => Some(format!(
            "Laufwerk \"{label}\" ist ausgefallen: {detail}. Der lokale Recovery-Status ist noch nicht verifiziert; der Cache bleibt bis zur erneuten Pruefung erhalten."
        )),
        crate::mount::MountStatus::Failed { detail } => Some(format!(
            "Laufwerk \"{label}\" konnte nicht bereitgestellt werden: {detail}. Der saubere Eintrag kann im Laufwerksmanager entfernt werden."
        )),
        crate::mount::MountStatus::RuntimeUnavailable { detail } => Some(format!(
            "Laufwerk \"{label}\" konnte nicht bereitgestellt werden: {detail}. Details stehen im Laufwerksmanager."
        )),
        _ => None,
    }
}

/// The first poll imports existing daemon state; it is not a fresh failure.
/// Keep its details visible in the manager and alert on later transitions or
/// explicit actions through mount_status_alert as before.
pub(super) fn has_mount_attention(mounts: &[crate::mount::MountSnapshot]) -> bool {
    mounts.iter().any(|mount| matches!(mount.status,
        crate::mount::MountStatus::Failed { .. }
        | crate::mount::MountStatus::Conflict { .. }
        | crate::mount::MountStatus::RuntimeUnavailable { .. }))
}

pub(super) fn mount_list_alert(
    previous: &[crate::mount::MountSnapshot],
    mounts: &[crate::mount::MountSnapshot],
    initial: bool,
) -> Option<String> {
    if initial { return None; }
    mounts.iter().find_map(|mount| {
        let known = previous.iter().find(|known| known.config.id == mount.config.id);
        mount_status_alert(known.map(|known| &known.status), mount)
    })
}

#[cfg(test)]
#[path = "mount_recovery_cache_task_tests.rs"]
mod task_tests;

pub(super) fn recovery_label(recovery: crate::mount::MountRecovery) -> &'static str {
    match recovery {
        crate::mount::MountRecovery::Clean => "Recovery: sauber",
        crate::mount::MountRecovery::Required => "Recovery: Aenderungen oder Konflikte offen",
        crate::mount::MountRecovery::Unknown => "Recovery: noch nicht verifiziert",
    }
}

pub(super) fn drive_selection_label(selection: crate::mount::DriveSelection) -> String {
    match selection {
        crate::mount::DriveSelection::Automatic => "Automatisch".into(),
        crate::mount::DriveSelection::Letter(letter) => letter.to_string(),
    }
}

pub(super) fn status_label(status: &crate::mount::MountStatus) -> String {
    match status {
        crate::mount::MountStatus::Unmounted => "Nicht eingebunden".into(),
        crate::mount::MountStatus::Mounting => "Wird eingebunden ...".into(),
        crate::mount::MountStatus::Mounted { drive } => format!("Eingebunden als {drive}"),
        crate::mount::MountStatus::Unmounting => "Wird ausgeworfen ...".into(),
        crate::mount::MountStatus::RuntimeUnavailable { detail } => {
            format!("Dokany fehlt: {detail}")
        }
        crate::mount::MountStatus::Conflict { path, detail, .. } => {
            format!("Pruefung offen bei {path}: {detail}")
        }
        crate::mount::MountStatus::Failed { detail } => format!("Fehler: {detail}"),
    }
}

pub(super) fn bounded_label(label: &str) -> String {
    let label = label.trim();
    let label = if label.is_empty() {
        "Smart Explorer"
    } else {
        label
    };
    label.chars().take(128).collect()
}
