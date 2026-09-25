//! GUI entry points for removing a Direct peer, a room, a saved connection or
//! a leftover authorization. The transactions live in `crate::share::removal`;
//! the GUI applies their result, cleans every derived store
//! (`cleanup_after_removal`) and reports what happened.
use crate::app::connection_cleanup::CleanupReport;
use crate::share::removal::{self, ProfileRemoval};

use super::*;

impl App {
    fn share_home() -> String {
        dirs_home().to_string_lossy().replace('\\', "/")
    }

    fn report_cleanup(&mut self, headline: String, report: &CleanupReport) {
        let summary = removal::cleanup_notice(headline, report);
        self.append_share_diag(summary.notice.clone());
        self.notice = Some((summary.notice, std::time::Instant::now()));
        if let Some(error) = summary.error {
            self.error_msg = Some(error);
        }
    }

    /// Adopt a persisted removal: profiles, warning, derived-store cleanup or
    /// a plain notice, then reconfigure the Share service when it changed.
    fn apply_profile_removal(&mut self, removal: ProfileRemoval, poll_now: bool) {
        self.share_profiles = removal.profiles;
        if let Some(warning) = removal.warning {
            self.error_msg = Some(warning);
        }
        match removal.scope {
            Some(scope) => {
                let report = self.cleanup_after_removal(&scope);
                self.report_cleanup(removal.headline, &report);
            }
            None => {
                self.append_share_diag(removal.headline.clone());
                self.notice = Some((removal.headline, std::time::Instant::now()));
            }
        }
        if removal.changed {
            let _ = self.configure_share_service();
            if poll_now {
                self.share_next_poll_at = Instant::now();
            }
        }
    }

    /// "Entfernen" on a Direct device: contact, secret, grant, requests and a
    /// durable denial of automatic re-pairing, then favourites, folder
    /// preferences, mounts and open tabs.
    pub(in crate::app) fn remove_direct_peer_completely(&mut self, contact_id: &str) {
        match removal::remove_direct_peer(
            Some(Self::share_home()),
            &self.share_profiles,
            contact_id,
        ) {
            Ok(removal) => self.apply_profile_removal(removal, true),
            Err(error) => self.error_msg = Some(error),
        }
    }

    /// "Eintrag loeschen" on an authorization: the grant and its requests go,
    /// the device is denied automatic re-pairing until paired again.
    pub(in crate::app) fn delete_direct_grant_entry(&mut self, device_id: &str) {
        match removal::delete_direct_grant(Some(Self::share_home()), device_id) {
            Ok(removal) => self.apply_profile_removal(removal, true),
            Err(error) => self.error_msg = Some(error),
        }
    }

    /// "Erneut zulassen" on a removed device.
    pub(in crate::app) fn readmit_removed_device(&mut self, device_id: &str) {
        match removal::readmit_removed_device(Some(Self::share_home()), device_id) {
            Ok(removal) => self.apply_profile_removal(removal, false),
            Err(error) => self.error_msg = Some(error),
        }
    }

    /// Room removal plus every derived store of that room.
    pub(in crate::app) fn remove_room_completely(&mut self, room_profile_id: &str) {
        match removal::remove_room(
            Some(Self::share_home()),
            &self.share_profiles,
            room_profile_id,
        ) {
            Ok(removal) => self.apply_profile_removal(removal, false),
            Err(error) => self.error_msg = Some(error),
        }
    }

    /// Saved SFTP/FTP/WebDAV/UNC connection removal plus its derived stores.
    pub(in crate::app) fn remove_saved_connection_completely(&mut self, account: &str) {
        match removal::remove_saved_connection(&self.saved_connections, account) {
            Ok(removal) => {
                self.saved_connections = removal.connections;
                match removal.scope {
                    Some(scope) => {
                        let report = self.cleanup_after_removal(&scope);
                        self.report_cleanup(removal.headline, &report);
                    }
                    None => {
                        self.notice = Some((removal.headline, std::time::Instant::now()));
                    }
                }
            }
            Err(error) => self.error_msg = Some(error),
        }
    }
}
