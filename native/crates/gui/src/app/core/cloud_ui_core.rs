use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn drain_cloud_auth(&mut self) {
        let result = match self.cloud_auth_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.cloud_auth_rx = None;
                self.cloud_authing = false;
                self.error_msg = Some("Cloud-Anmeldung wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.cloud_auth_rx = None;
        self.cloud_authing = false;
        match result {
            Ok(()) => {
                self.notice = Some((
                    "✓ Google Drive verbunden".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => self.error_msg = Some(format!("Cloud-Anmeldung: {error}")),
        }
    }

    pub(in crate::app) fn open_gdrive_browse(&mut self) {
        self.open_gdrive_location("gdrive:///");
    }

    /// Reopen a Drive locator through the shared endpoint resolver. Preserve
    /// literal subpaths, including percent signs and Unicode names.
    pub(in crate::app) fn open_gdrive_location(&mut self, locator: &str) {
        let auth = crate::cloud::auth_status(crate::cloud::Provider::GDrive);
        if !auth.signed_in {
            self.error_msg = Some(auth.message);
            return;
        }
        if self.connecting || self.connect_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let locator = locator.to_string();
        let spawn = std::thread::Builder::new()
            .name("gdrive-open".into())
            .spawn(move || {
                let _ = tx.send(open_gdrive_location_result(&locator));
            });
        match spawn {
            Ok(_) => {
                self.connect_rx = Some(rx);
                self.connecting = true;
                self.notice = Some((
                    "Verbinde mit Google Drive…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Google-Drive-Verbindung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn picker_open_gdrive(&mut self) {
        if self
            .picker
            .as_ref()
            .map(|picker| picker.connecting || picker.connect_rx.is_some())
            .unwrap_or(true)
        {
            return;
        }
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("gdrive-pick".into())
            .spawn(move || {
                let _ = tx.send(open_gdrive_result());
            });
        match spawn {
            Ok(_) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.backend = None;
                    picker.list_rx = None;
                    picker.listing = false;
                    picker.entries.clear();
                    picker.cwd.clear();
                    picker.error = None;
                    picker.connect_rx = Some(rx);
                    picker.connecting = true;
                    picker.is_remote = true;
                    picker.endpoint_prefix = "gdrive://".to_string();
                    picker.conn_label = "Google Drive".to_string();
                }
            }
            Err(error) => {
                let detail = format!("Google-Drive-Auswahl konnte nicht gestartet werden: {error}");
                if let Some(picker) = self.picker.as_mut() {
                    picker.connect_rx = None;
                    picker.connecting = false;
                    picker.error = Some(detail.clone());
                }
                self.error_msg = Some(detail);
            }
        }
    }
}

fn open_gdrive_result() -> crate::connect::ConnectResult {
    open_gdrive_location_result("gdrive:///")
}

fn open_gdrive_location_result(locator: &str) -> crate::connect::ConnectResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::connect::resolve_endpoint(locator)
    })) {
        Ok(Ok((backend, root))) => crate::connect::ConnectResult::Ok(crate::connect::Connected {
            remote: Some(crate::connect::RemoteState {
                backend,
                label: "Google Drive".to_string(),
                agent_version: None,
                zip_return: None,
                sftp: None,
                account: None,
                endpoint_prefix: Some("gdrive://".to_string()),
            }),
            net: None,
            target: root,
            label: "Google Drive".to_string(),
        }),
        Ok(Err(error)) => crate::connect::ConnectResult::Err(error),
        Err(_) => crate::connect::ConnectResult::Err(
            "Google-Drive-Verbindung wurde unerwartet beendet.".to_string(),
        ),
    }
}
