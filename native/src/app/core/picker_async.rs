use super::prelude::*;
use super::*;

impl App {
    /// List the current picker folder off the UI thread; remote latency must
    /// never freeze pointer, keyboard, or cancellation handling.
    pub(in crate::app) fn picker_list(&mut self) {
        let (backend, cwd) = match &self.picker {
            Some(picker) => match &picker.backend {
                Some(backend) => (backend.clone(), ensure_dir_root(&picker.cwd)),
                None => return,
            },
            None => return,
        };
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("picker-list".into())
            .spawn(move || {
                let result = backend
                    .list_dir(&cwd)
                    .map_err(|error| error.to_string())
                    .and_then(checked_picker_dirs);
                let _ = tx.send(result);
            });
        if let Some(picker) = self.picker.as_mut() {
            picker.entries.clear();
            picker.selected = None;
            picker.error = None;
            match spawn {
                Ok(_) => {
                    picker.list_rx = Some(rx);
                    picker.listing = true;
                }
                Err(error) => {
                    picker.list_rx = None;
                    picker.listing = false;
                    picker.error = Some(format!(
                        "Ordnerliste konnte nicht gestartet werden: {error}"
                    ));
                }
            }
        }
    }

    pub(in crate::app) fn drain_picker_list(&mut self) {
        let result = match self
            .picker
            .as_ref()
            .and_then(|picker| picker.list_rx.as_ref())
            .map(|rx| rx.try_recv())
        {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.list_rx = None;
                    picker.listing = false;
                    picker.error = Some("Ordnerlisten-Worker wurde ohne Ergebnis beendet.".into());
                }
                return;
            }
        };
        if let Some(picker) = self.picker.as_mut() {
            picker.list_rx = None;
            picker.listing = false;
            match result {
                Ok(entries) => picker.entries = entries,
                Err(error) => picker.error = Some(error),
            }
        }
    }

    pub(in crate::app) fn picker_open_connection(
        &mut self,
        connection: &crate::creds::SavedConnection,
    ) {
        let form = crate::connect::ConnectForm::from_saved(connection);
        let secret = crate::creds::get_secret(&connection.account());
        let result = crate::connect::spawn_connect(form, secret);
        if let Some(picker) = self.picker.as_mut() {
            picker.error = None;
            picker.conn_label = connection.display();
            picker.is_remote = connection.protocol.is_url();
            picker.endpoint_prefix = if connection.protocol.is_url() {
                format!(
                    "{}://{}@{}:{}",
                    connection.protocol.as_str(),
                    connection.user,
                    connection.host,
                    connection.port
                )
            } else {
                String::new()
            };
            match result {
                Ok(rx) => {
                    picker.connect_rx = Some(rx);
                    picker.connecting = true;
                }
                Err(error) => {
                    picker.connect_rx = None;
                    picker.connecting = false;
                    picker.error = Some(error);
                }
            }
        }
    }

    pub(in crate::app) fn drain_picker_connect(&mut self) {
        let message = match self
            .picker
            .as_ref()
            .and_then(|picker| picker.connect_rx.as_ref())
            .map(|rx| rx.try_recv())
        {
            Some(Ok(message)) => message,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.connect_rx = None;
                    picker.connecting = false;
                    picker.error = Some("Verbindungs-Thread wurde ohne Ergebnis beendet.".into());
                }
                return;
            }
        };
        let mut list = false;
        if let Some(picker) = self.picker.as_mut() {
            picker.connect_rx = None;
            picker.connecting = false;
            match message {
                crate::connect::ConnectResult::Ok(connected) => {
                    if let Some(remote) = connected.remote {
                        picker.backend = Some(cache_remote(remote.backend));
                        picker.is_remote = true;
                    } else {
                        picker.backend =
                            Some(Arc::new(crate::vfs::LocalBackend::new(&connected.target)));
                        picker.is_remote = false;
                        picker.endpoint_prefix.clear();
                    }
                    picker.cwd = connected.target;
                    list = true;
                }
                crate::connect::ConnectResult::Err(error) => {
                    picker.error = Some(format!("Verbindung fehlgeschlagen: {error}"));
                }
            }
        }
        if list {
            self.picker_list();
        }
    }
}

fn checked_picker_dirs(metas: Vec<crate::vfs::VfsMeta>) -> Result<Vec<String>, String> {
    const MAX_ENTRIES: usize = 1_000_000;
    const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;
    if metas.len() > MAX_ENTRIES {
        return Err(format!(
            "Ordnerliste überschreitet das Limit von {MAX_ENTRIES} Einträgen"
        ));
    }
    let mut seen = std::collections::HashSet::with_capacity(metas.len().min(16_384));
    let mut text_bytes = 0usize;
    let mut dirs = Vec::new();
    for meta in metas {
        crate::vfs::validate_child_name(&meta.name).map_err(|error| error.to_string())?;
        text_bytes = text_bytes
            .checked_add(meta.name.len())
            .filter(|bytes| *bytes <= MAX_TEXT_BYTES)
            .ok_or_else(|| "Ordnerliste überschreitet das Textlimit von 128 MiB".to_string())?;
        if !seen.insert(meta.name.clone()) {
            return Err(format!("Backend lieferte den Namen doppelt: {}", meta.name));
        }
        if meta.is_dir && !meta.is_symlink {
            dirs.push(meta.name);
        }
    }
    dirs.sort_unstable();
    Ok(dirs)
}
