//! What still uses the previous address after an edit moved a saved
//! connection (host, port, user or root changed). As after a removal
//! (`cleanup.rs`), sync setups are the user's own work and are only named,
//! never rewritten: a changed host may be another server, and a rewritten
//! endpoint would be compared against the baseline of the old one. Mounts
//! keep their open session until they reconnect and then no longer find the
//! connection.
use crate::creds::SavedConnection;

use super::location::saved_location;
use super::removal_scope::RemovedEndpointScope;

/// One warning per kind of reference to `previous` that no saved connection
/// resolves any more now that `saved` replaced it.
pub(super) fn references_left_behind(
    previous: &SavedConnection,
    saved: &SavedConnection,
) -> Vec<String> {
    let old = RemovedEndpointScope::for_saved_connection(previous);
    let new = RemovedEndpointScope::for_saved_connection(saved);
    let mut warnings = Vec::new();
    if old.prefixes != new.prefixes {
        match crate::syncjobs::load() {
            Ok(jobs) => {
                let connections = crate::creds::load_connections();
                let names = jobs_left_behind(
                    jobs.iter()
                        .map(|job| (job.name.as_str(), job.source.as_str(), job.target.as_str())),
                    &connections,
                    &old,
                    &new,
                );
                if !names.is_empty() {
                    warnings.push(format!(
                        "Sync-Setups nutzen weiter die alte Adresse {} (unverändert; im Editor auf die neue umstellen): {}",
                        old.prefixes.join(", "),
                        names.join(", ")
                    ));
                }
            }
            Err(error) => warnings.push(format!("Sync-Setups nicht geprüft: {error}")),
        }
    }
    // Mounts live only in a running worker; asking a stopped one would start it.
    if crate::daemon::is_running() {
        match crate::daemon::list_mounts() {
            Ok(mounts) => {
                let labels = mounts
                    .iter()
                    .filter(|snapshot| old.matches_mount(&snapshot.config.source))
                    .map(|snapshot| snapshot.config.label.clone())
                    .collect::<Vec<_>>();
                if !labels.is_empty() {
                    warnings.push(format!(
                        "Laufwerke der alten Verbindung finden sie nach einer Trennung nicht mehr (neu einrichten): {}",
                        labels.join(", ")
                    ));
                }
            }
            Err(error) => warnings.push(format!("Laufwerke nicht geprüft: {error}")),
        }
    }
    warnings
}

/// Names of the jobs (`name`, `source`, `target`) with an endpoint on the old
/// address that neither the new address nor another saved connection serves.
fn jobs_left_behind<'a>(
    jobs: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
    connections: &[SavedConnection],
    old: &RemovedEndpointScope,
    new: &RemovedEndpointScope,
) -> Vec<String> {
    let left_behind = |endpoint: &str| {
        old.matches_key(endpoint)
            && !new.matches_key(endpoint)
            && !(crate::connect::is_remote_url(endpoint)
                && saved_location(connections, endpoint).is_some())
    };
    jobs.filter(|(_, source, target)| left_behind(*source) || left_behind(*target))
        .map(|(name, _, _)| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creds::{AuthKind, Protocol};

    fn connection(host: &str, root: &str) -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: host.into(),
            port: 22,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: root.into(),
            label: "nas".into(),
            use_agent: false,
        }
    }

    const JOBS: [(&str, &str, &str); 3] = [
        ("Docs", "/home/alice/Docs", "sftp://alice@nas:22/srv/docs"),
        ("Fotos", "sftp://alice@nas:22/srv/fotos", "/home/alice/Fotos"),
        ("Lokal", "/home/alice/a", "/home/alice/b"),
    ];

    #[test]
    fn luecken_task_connection_move_names_the_sync_setups_left_on_the_old_address() {
        let previous = connection("nas", "/srv");
        let saved = connection("nas.lan", "/srv");
        let old = RemovedEndpointScope::for_saved_connection(&previous);
        let new = RemovedEndpointScope::for_saved_connection(&saved);
        let names = jobs_left_behind(JOBS.into_iter(), std::slice::from_ref(&saved), &old, &new);
        assert_eq!(names, ["Docs", "Fotos"]);
        // Another saved connection still serving the old address keeps them.
        let other = connection("nas", "/srv/fotos");
        let names = jobs_left_behind(JOBS.into_iter(), &[saved, other], &old, &new);
        assert!(names.is_empty(), "{names:?}");
    }

    #[test]
    fn luecken_task_connection_move_of_the_root_only_keeps_the_sync_setups() {
        let previous = connection("nas", "/srv");
        let saved = connection("nas", "/backup");
        let old = RemovedEndpointScope::for_saved_connection(&previous);
        let new = RemovedEndpointScope::for_saved_connection(&saved);
        // Sync endpoints carry the address without the start path.
        assert_eq!(old.prefixes, new.prefixes);
        let names = jobs_left_behind(JOBS.into_iter(), std::slice::from_ref(&saved), &old, &new);
        assert!(names.is_empty(), "{names:?}");
        // Drives are bound to the whole account, start path included.
        assert_ne!(old.mounts, new.mounts);
    }
}
