//! GUI entry points for removing a Direct peer, a room, a saved connection or
//! a leftover authorization. Each one persists the profile transaction, cleans
//! every derived store (`connection_cleanup`) and reports what happened.
use crate::app::connection_cleanup::{CleanupReport, RemovedEndpointScope};

use super::*;

impl App {
    fn share_home() -> String {
        dirs_home().to_string_lossy().replace('\\', "/")
    }

    fn report_cleanup(&mut self, headline: String, report: &CleanupReport) {
        let mut lines = vec![headline];
        lines.extend(report.summary_lines());
        let notice = lines.join(" · ");
        self.append_share_diag(notice.clone());
        self.notice = Some((notice, std::time::Instant::now()));
        if report.mount_error.is_some() || !report.file_errors.is_empty() {
            let mut problems = report.file_errors.clone();
            if let Some(error) = &report.mount_error {
                problems.push(format!("Laufwerke nicht geprueft: {error}"));
            }
            self.error_msg = Some(problems.join("; "));
        }
    }

    /// "Entfernen" on a Direct device: contact, secret, grant, requests and a
    /// durable denial of automatic re-pairing, then favourites, folder
    /// preferences, mounts and open tabs.
    pub(in crate::app) fn remove_direct_peer_completely(&mut self, contact_id: &str) {
        let display_name = self
            .share_profiles
            .direct_contacts
            .iter()
            .find(|contact| contact.id == contact_id)
            .map(|contact| contact.display_name.clone())
            .unwrap_or_else(|| contact_id.to_string());
        match crate::share::ShareProfiles::forget_direct_peer_persisted(
            Some(Self::share_home()),
            contact_id,
        ) {
            Ok((profiles, change, forgotten)) => {
                self.share_profiles = profiles;
                if let Some(warning) = change.cleanup_warning {
                    self.error_msg = Some(warning);
                }
                let scope = RemovedEndpointScope::for_direct_contact(contact_id);
                let report = self.cleanup_after_removal(&scope);
                let headline = match forgotten {
                    Some(peer) if peer.identity.is_some() => format!(
                        "{display_name} entfernt: {} Autorisierung(en), {} Anfrage(n); automatische Wiederkopplung gesperrt",
                        peer.grants_removed,
                        peer.requests_removed + peer.legacy_requests_removed
                    ),
                    Some(_) => format!("{display_name} entfernt"),
                    None => format!("{display_name} war bereits entfernt"),
                };
                self.report_cleanup(headline, &report);
                if change.changed {
                    let _ = self.configure_share_service();
                    self.share_next_poll_at = Instant::now();
                }
            }
            Err(error) => {
                self.error_msg = Some(format!("Direktgeraet nicht entfernt: {error}"));
            }
        }
    }

    /// "Eintrag loeschen" on an authorization: the grant and its requests go,
    /// the device is denied automatic re-pairing until paired again.
    pub(in crate::app) fn delete_direct_grant_entry(&mut self, device_id: &str) {
        match crate::share::ShareProfiles::delete_direct_grant_persisted(
            Some(Self::share_home()),
            device_id,
        ) {
            Ok((profiles, change)) => {
                self.share_profiles = profiles;
                let headline = if change.changed {
                    format!(
                        "Autorisierung {device_id} geloescht; automatische Wiederkopplung gesperrt"
                    )
                } else {
                    format!("Autorisierung {device_id} war bereits geloescht")
                };
                self.append_share_diag(headline.clone());
                self.notice = Some((headline, std::time::Instant::now()));
                if change.changed {
                    let _ = self.configure_share_service();
                    self.share_next_poll_at = Instant::now();
                }
            }
            Err(error) => {
                self.error_msg = Some(format!("Autorisierung nicht geloescht: {error}"));
            }
        }
    }

    /// "Erneut zulassen" on a removed device.
    pub(in crate::app) fn readmit_removed_device(&mut self, device_id: &str) {
        match crate::share::ShareProfiles::readmit_removed_direct_peer_persisted(
            Some(Self::share_home()),
            device_id,
        ) {
            Ok((profiles, change)) => {
                self.share_profiles = profiles;
                let headline = if change.changed {
                    format!("{device_id} darf sich wieder automatisch koppeln")
                } else {
                    format!("{device_id} war nicht gesperrt")
                };
                self.append_share_diag(headline.clone());
                self.notice = Some((headline, std::time::Instant::now()));
                if change.changed {
                    let _ = self.configure_share_service();
                }
            }
            Err(error) => {
                self.error_msg = Some(format!("Sperre nicht aufgehoben: {error}"));
            }
        }
    }

    /// Room removal plus every derived store of that room.
    pub(in crate::app) fn remove_room_completely(&mut self, room_profile_id: &str) {
        let room_id = self
            .share_profiles
            .rooms
            .iter()
            .find(|room| room.id == room_profile_id)
            .map(|room| room.room_id.clone())
            .unwrap_or_default();
        match crate::share::ShareProfiles::remove_room_persisted(
            Some(Self::share_home()),
            room_profile_id,
        ) {
            Ok((profiles, change)) => {
                self.share_profiles = profiles;
                if let Some(warning) = change.cleanup_warning {
                    self.error_msg = Some(warning);
                }
                let scope = RemovedEndpointScope::for_room(room_profile_id, &room_id);
                let report = self.cleanup_after_removal(&scope);
                self.report_cleanup("Raum entfernt".to_string(), &report);
                if change.changed {
                    let _ = self.configure_share_service();
                }
            }
            Err(error) => self.error_msg = Some(format!("Raum nicht entfernt: {error}")),
        }
    }

    /// Saved SFTP/FTP/WebDAV/UNC connection removal plus its derived stores.
    pub(in crate::app) fn remove_saved_connection_completely(&mut self, account: &str) {
        let connection = self
            .saved_connections
            .iter()
            .find(|connection| connection.account() == account)
            .cloned();
        match crate::creds::remove_connection(account) {
            Ok(()) => {
                self.saved_connections = crate::creds::load_connections();
                let report = connection.as_ref().map(|connection| {
                    let scope = RemovedEndpointScope::for_saved_connection(connection);
                    self.cleanup_after_removal(&scope)
                });
                match report {
                    Some(report) => {
                        self.report_cleanup("Gespeicherte Verbindung entfernt".to_string(), &report)
                    }
                    None => {
                        self.notice = Some((
                            "Gespeicherte Verbindung entfernt".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Gespeicherte Verbindung konnte nicht entfernt werden: {error}"
                ));
            }
        }
    }
}
