use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn picker_open_peer(&mut self, label: String, target: crate::share::PeerOpenTarget) {
        let (tx, rx) = unbounded();
        let prefix = target.endpoint_prefix();
        let spawn = std::thread::Builder::new().name("picker-share".into()).spawn(move || {
            let result = match crate::daemon::open_share_backend(target) {
                Ok((_, backend, _)) => crate::connect::ConnectResult::Ok(crate::connect::Connected {
                    remote: Some(crate::connect::RemoteState {
                        backend, label: label.clone(), agent_version: None, zip_return: None,
                        sftp: None, account: None, endpoint_prefix: Some(prefix),
                    }), net: None, target: "/".into(), label,
                }),
                Err(error) => crate::connect::ConnectResult::Err(error),
            };
            let _ = tx.send(result);
        });
        if let Some(picker) = self.picker.as_mut() {
            picker.backend = None;
            picker.list_rx = None;
            picker.listing = false;
            picker.cwd.clear();
            picker.entries.clear();
            picker.endpoint_prefix.clear();
            picker.error = None;
            match spawn {
                Ok(_) => { picker.connect_rx = Some(rx); picker.connecting = true; }
                Err(error) => {
                    picker.connect_rx = None;
                    picker.connecting = false;
                    picker.error = Some(format!("Share-Verbindung konnte nicht starten: {error}"));
                }
            }
        }
    }
}
