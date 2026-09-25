//! Removal cascades without UI: removing a Direct peer, a leftover
//! authorization, a device denial, a room or a saved connection. Each call
//! persists its transaction and returns what the caller applies next: the
//! committed state, the headline, and the scope of derived stores to clean
//! with `connect::cleanup_removed_endpoint_state` (favourites, folder
//! preferences, mounts) before closing views of that endpoint.
use crate::connect::{CleanupReport, RemovedEndpointScope};
use crate::creds::SavedConnection;
use crate::share::ShareProfiles;

/// A persisted Share profile removal.
pub struct ProfileRemoval {
    pub profiles: ShareProfiles,
    /// The profiles changed: reconfigure the Share service.
    pub changed: bool,
    /// The removal persisted, but a secondary cleanup (a stored secret) failed.
    pub warning: Option<String>,
    pub headline: String,
    /// Derived stores to clean; `None` when the removal leaves none behind.
    pub scope: Option<RemovedEndpointScope>,
}

/// A removed saved connection (SFTP/FTP/WebDAV/UNC).
pub struct SavedConnectionRemoval {
    /// The saved connections after the removal.
    pub connections: Vec<SavedConnection>,
    pub headline: String,
    /// `None` when the connection was not among the known ones.
    pub scope: Option<RemovedEndpointScope>,
}

/// What to show after a cleanup: one notice line and, when a store could not
/// be cleaned, an error line.
pub struct CleanupNotice {
    pub notice: String,
    pub error: Option<String>,
}

/// "Entfernen" on a Direct device: contact, secret, grant, requests and a
/// durable denial of automatic re-pairing.
pub fn remove_direct_peer(
    default_home: Option<String>,
    current: &ShareProfiles,
    contact_id: &str,
) -> Result<ProfileRemoval, String> {
    let display_name = current
        .direct_contacts
        .iter()
        .find(|contact| contact.id == contact_id)
        .map(|contact| contact.display_name.clone())
        .unwrap_or_else(|| contact_id.to_string());
    let (profiles, change, forgotten) =
        ShareProfiles::forget_direct_peer_persisted(default_home, contact_id)
            .map_err(|error| format!("Direktgeraet nicht entfernt: {error}"))?;
    let headline = match forgotten {
        Some(peer) if peer.identity.is_some() => format!(
            "{display_name} entfernt: {} Autorisierung(en), {} Anfrage(n); automatische Wiederkopplung gesperrt",
            peer.grants_removed,
            peer.requests_removed + peer.legacy_requests_removed
        ),
        Some(_) => format!("{display_name} entfernt"),
        None => format!("{display_name} war bereits entfernt"),
    };
    Ok(ProfileRemoval {
        profiles,
        changed: change.changed,
        warning: change.cleanup_warning,
        headline,
        scope: Some(RemovedEndpointScope::for_direct_contact(contact_id)),
    })
}

/// "Eintrag loeschen" on an authorization: the grant and its requests go,
/// the device is denied automatic re-pairing until paired again.
pub fn delete_direct_grant(
    default_home: Option<String>,
    device_id: &str,
) -> Result<ProfileRemoval, String> {
    let (profiles, change) = ShareProfiles::delete_direct_grant_persisted(default_home, device_id)
        .map_err(|error| format!("Autorisierung nicht geloescht: {error}"))?;
    let headline = if change.changed {
        format!("Autorisierung {device_id} geloescht; automatische Wiederkopplung gesperrt")
    } else {
        format!("Autorisierung {device_id} war bereits geloescht")
    };
    Ok(ProfileRemoval {
        profiles,
        changed: change.changed,
        warning: change.cleanup_warning,
        headline,
        scope: None,
    })
}

/// "Erneut zulassen" on a removed device.
pub fn readmit_removed_device(
    default_home: Option<String>,
    device_id: &str,
) -> Result<ProfileRemoval, String> {
    let (profiles, change) =
        ShareProfiles::readmit_removed_direct_peer_persisted(default_home, device_id)
            .map_err(|error| format!("Sperre nicht aufgehoben: {error}"))?;
    let headline = if change.changed {
        format!("{device_id} darf sich wieder automatisch koppeln")
    } else {
        format!("{device_id} war nicht gesperrt")
    };
    Ok(ProfileRemoval {
        profiles,
        changed: change.changed,
        warning: change.cleanup_warning,
        headline,
        scope: None,
    })
}

/// Room removal; the scope covers both the profile id and the wire room id.
pub fn remove_room(
    default_home: Option<String>,
    current: &ShareProfiles,
    room_profile_id: &str,
) -> Result<ProfileRemoval, String> {
    let room_id = current
        .rooms
        .iter()
        .find(|room| room.id == room_profile_id)
        .map(|room| room.room_id.clone())
        .unwrap_or_default();
    let (profiles, change) = ShareProfiles::remove_room_persisted(default_home, room_profile_id)
        .map_err(|error| format!("Raum nicht entfernt: {error}"))?;
    Ok(ProfileRemoval {
        profiles,
        changed: change.changed,
        warning: change.cleanup_warning,
        headline: "Raum entfernt".to_string(),
        scope: Some(RemovedEndpointScope::for_room(room_profile_id, &room_id)),
    })
}

/// Saved SFTP/FTP/WebDAV/UNC connection removal.
pub fn remove_saved_connection(
    current: &[SavedConnection],
    account: &str,
) -> Result<SavedConnectionRemoval, String> {
    let scope = current
        .iter()
        .find(|connection| connection.account() == account)
        .map(RemovedEndpointScope::for_saved_connection);
    crate::creds::remove_connection(account).map_err(|error| {
        format!("Gespeicherte Verbindung konnte nicht entfernt werden: {error}")
    })?;
    Ok(SavedConnectionRemoval {
        connections: crate::creds::load_connections(),
        headline: "Gespeicherte Verbindung entfernt".to_string(),
        scope,
    })
}

/// The headline and every cleanup line as one notice; unclean stores as error.
pub fn cleanup_notice(headline: String, report: &CleanupReport) -> CleanupNotice {
    let mut lines = vec![headline];
    lines.extend(report.summary_lines());
    let error = (report.mount_error.is_some() || !report.file_errors.is_empty()).then(|| {
        let mut problems = report.file_errors.clone();
        if let Some(error) = &report.mount_error {
            problems.push(format!("Laufwerke nicht geprueft: {error}"));
        }
        problems.join("; ")
    });
    CleanupNotice {
        notice: lines.join(" · "),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::cleanup_notice;
    use crate::connect::CleanupReport;

    #[test]
    fn android_task_cleanup_notice_joins_lines_and_reports_unclean_stores() {
        let mut report = CleanupReport {
            favorites_removed: 2,
            ..CleanupReport::default()
        };
        let clean = cleanup_notice("Raum entfernt".to_string(), &report);
        assert_eq!(clean.notice, "Raum entfernt · 2 Favorit(en) entfernt");
        assert!(clean.error.is_none());

        report
            .file_errors
            .push("Favoriten bereinigen: denied".to_string());
        report.mount_error = Some("daemon offline".to_string());
        let unclean = cleanup_notice("Raum entfernt".to_string(), &report);
        assert_eq!(
            unclean.error.as_deref(),
            Some("Favoriten bereinigen: denied; Laufwerke nicht geprueft: daemon offline")
        );
    }
}
