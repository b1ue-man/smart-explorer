#[path = "share_diagnostics_ui.rs"]
mod diagnostics_ui;
#[path = "share_exports_ui.rs"]
mod exports_ui;
#[path = "share_rooms_ui.rs"]
mod rooms_ui;
#[path = "share_direct_ui.rs"]
mod direct_ui;
#[path = "share_window_ui.rs"]
mod window_ui;
use super::prelude::*;
use super::*;

#[path = "share_drain.rs"]
mod drain;
#[path = "share_helpers.rs"]
mod helpers;
#[path = "share_identity_rotation.rs"]
mod identity_rotation;
#[path = "share_lan_ui.rs"]
mod lan_ui;
#[path = "share_lan_uplink_ui.rs"]
mod lan_uplink_ui;
#[path = "share_legacy_lifecycle_ui.rs"]
mod legacy_lifecycle_ui;
#[path = "share_lifecycle_ui.rs"]
mod lifecycle_ui;
#[path = "share_navigation.rs"]
mod navigation;
#[path = "share_poll_status.rs"]
mod poll_status;
#[path = "share_profile_cache.rs"]
mod profile_cache;
#[path = "share_profile_edits.rs"]
mod profile_edits;
#[path = "share_removal_ui.rs"]
mod removal_ui;
#[path = "share_removed_devices_ui.rs"]
mod removed_devices_ui;
#[path = "share_lifecycle_view.rs"]
mod share_lifecycle_view;

use helpers::*;

const SHARE_ACTIVE_POLL: std::time::Duration = std::time::Duration::from_millis(300);
const SHARE_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(900);
const SHARE_DIAG_MAX_BYTES: usize = 48 * 1024;

impl App {
    pub(in crate::app) fn ensure_share(&mut self) -> bool {
        self.share_manual_stop = false;
        if let Some(svc) = self.share.take() {
            if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                self.append_share_diag(format!("Lokalen Share-Dienst stoppen: {error}"));
            }
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Share-Server eingetragen".to_string();
            return false;
        }
        if self.share_identity.is_none() {
            match crate::share::ShareIdentity::load_or_create(default_device_name()) {
                Ok(identity) => {
                    self.share_identity = Some(identity);
                    self.share_identity_error = None;
                }
                Err(error) => {
                    self.share_identity_error = Some(error.clone());
                    self.share_status = format!("Share-Identitaet nicht verfuegbar: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        if self.share_profiles_error.is_some() {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::load_checked(Some(default_home)) {
                Ok(profiles) => {
                    self.share_profiles = profiles;
                    self.share_profiles_error = None;
                }
                Err(error) => {
                    self.share_profiles_error = Some(error.clone());
                    self.share_status = format!("Share-Profile nicht verfuegbar: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        let Some(identity) = self.share_identity.as_mut() else {
            self.share_status = "Share-Identitaet nicht verfuegbar".into();
            self.error_msg = Some(self.share_status.clone());
            return false;
        };
        if let Err(error) = identity.set_device_name(self.share_device_draft.clone()) {
            self.share_status = format!("Share-Geraetename nicht gespeichert: {error}");
            self.error_msg = Some(self.share_status.clone());
            return false;
        }
        if !self.share_profiles.auto_connect {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |profiles| {
                profiles.auto_connect = true;
                Ok(())
            }) {
                Ok(committed) => self.share_profiles = committed,
                Err(error) => {
                    self.share_status = format!("Share-Profile nicht gespeichert: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        match crate::daemon::refresh_share_worker_checked() {
            Ok(true) => {
                self.share_status = format!("Share-Worker aktiv ({server})");
                true
            }
            Ok(false) => {
                self.share_status = "Share-Worker wurde nicht aktiv".into();
                self.error_msg = Some(self.share_status.clone());
                false
            }
            Err(error) => {
                self.share_status = format!("Share-Worker konnte nicht aktiviert werden: {error}");
                self.error_msg = Some(self.share_status.clone());
                false
            }
        }
    }

    pub(in crate::app) fn share_cmd(&mut self, c: crate::share::ShareCmd) -> bool {
        if matches!(&c, crate::share::ShareCmd::Stop) {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |profiles| {
                profiles.auto_connect = false;
                Ok(())
            }) {
                Ok(committed) => self.share_profiles = committed,
                Err(error) => {
                    self.share_status = format!("Trennen nicht gespeichert: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
            if let Err(error) = crate::daemon::send_share_command(c) {
                self.share_status = format!("Share-Worker Stop fehlgeschlagen: {error}");
                self.append_share_diag(self.share_status.clone());
                self.error_msg = Some(self.share_status.clone());
                return false;
            }
            if let Some(svc) = self.share.take() {
                if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                    self.append_share_diag(format!("Lokalen Share-Dienst stoppen: {error}"));
                }
            }
            self.share_manual_stop = true;
            self.share_worker_running = false;
            self.share_status = "Getrennt".to_string();
            return true;
        }
        if self.ensure_share() {
            if let Err(error) = crate::daemon::send_share_command(c) {
                self.share_status = format!("Share-Worker Fehler: {error}");
                self.append_share_diag(format!("Share-Worker Kommando: {error}"));
                self.error_msg = Some(self.share_status.clone());
                return false;
            }
            return true;
        }
        false
    }

    fn configure_share_service(&mut self) -> bool {
        match crate::daemon::refresh_share_worker_checked() {
            Ok(true) => true,
            Ok(false) => {
                self.error_msg = Some("Share-Worker wurde nicht aktiv".into());
                false
            }
            Err(error) => {
                self.error_msg = Some(format!("Share-Konfiguration zustellen: {error}"));
                false
            }
        }
    }

    fn commit_share_profiles(&mut self, previous: crate::share::ShareProfiles) -> bool {
        let edited = std::mem::replace(&mut self.share_profiles, previous.clone());
        let default_home = dirs_home().to_string_lossy().replace('\\', "/");
        match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |latest| {
            profile_edits::merge_user_edits(latest, &previous, &edited);
            Ok(())
        }) {
            Ok(committed) => {
                self.share_profiles = committed;
                self.configure_share_service()
            }
            Err(error) => {
                self.error_msg = Some(format!("Share-Profile speichern: {error}"));
                false
            }
        }
    }

    fn append_share_diag(&mut self, line: impl AsRef<str>) {
        self.share_diag_log.push_str(line.as_ref());
        if !self.share_diag_log.ends_with('\n') {
            self.share_diag_log.push('\n');
        }
        trim_share_diag_log(&mut self.share_diag_log);
    }

    fn should_log_share_op(&mut self) -> bool {
        if self.share_last_op_log_at.elapsed() < std::time::Duration::from_secs(2) {
            return false;
        }
        self.share_last_op_log_at = Instant::now();
        true
    }

    fn mark_opening_status(&mut self, status: crate::share::ShareStatus) {
        if let Some(target) = &self.share_opening {
            match target {
                crate::share::PeerOpenTarget::Direct { contact_id } => {
                    if let Some(c) = self
                        .share_profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| &c.id == contact_id)
                    {
                        c.status = status;
                    }
                }
                crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                    if let Some(r) = self
                        .share_profiles
                        .rooms
                        .iter_mut()
                        .find(|r| &r.id == room_id || &r.room_id == room_id)
                    {
                        if let Some(m) = r.members.iter_mut().find(|m| &m.device_id == device_id) {
                            m.status = status;
                        }
                    }
                }
            }
        }
    }

    pub(in crate::app) fn open_share_target(&mut self, target: crate::share::PeerOpenTarget) {
        if !self.ensure_share() {
            return;
        }
        if self.share_target_is_open(&target) {
            self.mark_target_status(&target, crate::share::ShareStatus::Connected);
            self.notice = Some((
                "Share-Verbindung ist bereits offen".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        if let crate::share::PeerOpenTarget::Direct { contact_id } = &target {
            if let Some(c) = self
                .share_profiles
                .direct_contacts
                .iter_mut()
                .find(|c| &c.id == contact_id)
            {
                if c.access_state != crate::share::DirectAccessState::Accepted {
                    c.status = crate::share::ShareStatus::WaitingForAccess;
                    self.notice = Some((
                        "Warte auf Freigabe am anderen Geraet".to_string(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
            }
        }
        self.share_opening = Some(target.clone());
        self.share_opening_origin = Some(self.share_open_context_key());
        self.mark_target_status(&target, crate::share::ShareStatus::Connecting);
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("share-open".into())
            .spawn(move || {
                let _ = tx.send(crate::daemon::open_share_backend(target));
            });
        match spawned {
            Ok(_) => self.share_open_rx = Some(rx),
            Err(e) => {
                let message = format!("Share-Verbindung konnte nicht gestartet werden: {e}");
                self.mark_opening_status(crate::share::ShareStatus::Failed(message.clone()));
                self.error_msg = Some(message.clone());
                self.append_share_diag(message);
                self.share_opening = None;
                self.share_opening_origin = None;
            }
        }
    }

    fn share_target_is_open(&self, target: &crate::share::PeerOpenTarget) -> bool {
        let Some(remote) = &self.remote else {
            return false;
        };
        remote
            .endpoint_prefix
            .as_deref()
            .map(|prefix| prefix == target.endpoint_prefix())
            .unwrap_or(false)
    }

    fn share_open_context_key(&self) -> String {
        let endpoint = self
            .remote
            .as_ref()
            .and_then(|r| r.endpoint_prefix.clone())
            .unwrap_or_else(|| "local".to_string());
        format!("{endpoint}|{}", self.root_path)
    }

    fn share_can_auto_open(&self) -> bool {
        self.root_path.is_empty()
            && self.remote.is_none()
            && self.net_conn.is_none()
            && self.share_opening.is_none()
            && !self.scan_running
    }

    fn mark_target_status(
        &mut self,
        target: &crate::share::PeerOpenTarget,
        status: crate::share::ShareStatus,
    ) {
        match target {
            crate::share::PeerOpenTarget::Direct { contact_id } => {
                if let Some(c) = self
                    .share_profiles
                    .direct_contacts
                    .iter_mut()
                    .find(|c| &c.id == contact_id)
                {
                    c.status = status;
                }
            }
            crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                if let Some(r) = self
                    .share_profiles
                    .rooms
                    .iter_mut()
                    .find(|r| &r.id == room_id || &r.room_id == room_id)
                {
                    if let Some(m) = r.members.iter_mut().find(|m| &m.device_id == device_id) {
                        m.status = status;
                    }
                }
            }
        }
    }

}
