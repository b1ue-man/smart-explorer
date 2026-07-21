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
            "Laufwerk \"{label}\": Der Remote-Status der Aenderung an {path} ist nicht abschliessend verifiziert; sie kann bereits gespeichert sein oder mit dem Remote-Stand kollidieren: {detail}. Die lokale Recovery-Kopie bleibt erhalten; Details stehen im Laufwerksmanager."
        )),
        crate::mount::MountStatus::Failed { detail } if mount.recovery_required => Some(format!(
            "Laufwerk \"{label}\" ist ausgefallen: {detail}. Nicht uebertragene Aenderungen bleiben im Recovery-Cache; Details stehen im Laufwerksmanager."
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
