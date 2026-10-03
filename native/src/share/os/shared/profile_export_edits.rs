//! Rebase only the exports the user edited. Concurrent additions and unrelated
//! rights changes remain in the latest persisted profile.
use crate::share::{ShareExportConfig, SharedConnection, SharedRoot};

pub(super) fn merge(latest: &mut ShareExportConfig, before: &ShareExportConfig,
    edited: &ShareExportConfig) -> Result<(), String> {
    if before == edited { return Ok(()); }
    if edited.include_connections {
        return Err("Gespeicherte Verbindungen muessen einzeln freigegeben werden".into());
    }
    if latest.include_connections {
        return Err("Alte Verbindungsfreigaben sind noch nicht migriert; bitte neu laden".into());
    }
    unique_roots(&before.roots)?;
    unique_roots(&edited.roots)?;
    unique_roots(&latest.roots)?;
    latest.roots.retain(|root| !before.roots.iter().any(|old| old.path == root.path)
        || edited.roots.iter().any(|new| new.path == root.path));
    for root in &edited.roots {
        let old = before.roots.iter().find(|old| old.path == root.path);
        match (old, latest.roots.iter_mut().find(|current| current.path == root.path)) {
            (Some(old), Some(current)) => {
                if root.label != old.label { current.label = root.label.clone(); }
                if root.access != old.access { current.access = root.access; }
                if root.allow_system_writes != old.allow_system_writes {
                    current.allow_system_writes = root.allow_system_writes;
                }
            }
            (Some(old), None) if root != old => return Err("Freigabe wurde inzwischen entfernt; bitte neu laden".into()),
            (None, Some(current)) if current != root => return Err("Freigabe wurde inzwischen anders hinzugefuegt; bitte neu laden".into()),
            (None, None) => latest.roots.push(root.clone()),
            _ => {}
        }
    }
    let old_order = before.roots.iter().map(|root| &root.path).collect::<Vec<_>>();
    let edited_order = edited.roots.iter().map(|root| &root.path).collect::<Vec<_>>();
    if old_order != edited_order {
        let order = edited.roots.iter().enumerate().map(|(index, root)| (root.path.as_str(), index))
            .collect::<std::collections::HashMap<_, _>>();
        latest.roots.sort_by_key(|root| order.get(root.path.as_str()).copied().unwrap_or(usize::MAX));
    }
    unique_connections(&before.shared_connections)?;
    unique_connections(&edited.shared_connections)?;
    unique_connections(&latest.shared_connections)?;
    latest.shared_connections.retain(|connection|
        !before.shared_connections.iter().any(|old| old.account == connection.account)
        || edited.shared_connections.iter().any(|new| new.account == connection.account));
    for connection in &edited.shared_connections {
        let old = before.shared_connections.iter().find(|old| old.account == connection.account);
        match (old, latest.shared_connections.iter_mut().find(|current| current.account == connection.account)) {
            (Some(old), Some(current)) if old.access != connection.access => current.access = connection.access,
            (Some(old), None) if old != connection => return Err("Verbindungsfreigabe wurde inzwischen entfernt; bitte neu laden".into()),
            (None, Some(current)) if current != connection => return Err("Verbindungsfreigabe wurde inzwischen anders hinzugefuegt; bitte neu laden".into()),
            (None, None) => latest.shared_connections.push(connection.clone()),
            _ => {}
        }
    }
    Ok(())
}

fn unique_roots(roots: &[SharedRoot]) -> Result<(), String> {
    let mut paths = std::collections::HashSet::new();
    if roots.iter().all(|root| paths.insert(root.path.as_str())) { Ok(()) }
    else { Err("Freigabepfad ist nicht eindeutig".into()) }
}

fn unique_connections(connections: &[SharedConnection]) -> Result<(), String> {
    let mut accounts = std::collections::HashSet::new();
    if connections.iter().all(|connection| accounts.insert(connection.account.as_str())) { Ok(()) }
    else { Err("Verbindungsfreigabe ist nicht eindeutig".into()) }
}
