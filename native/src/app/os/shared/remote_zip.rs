use super::*;

impl App {
    /// Open a local `.zip` as a browsable, read-only "remote" so it navigates
    /// like a folder (rscan walks the `ZipBackend`; ⏏ returns to the folder it
    /// lives in). Files inside open via the normal download-to-temp path.
    pub(in crate::app) fn open_zip(&mut self, zip_path: &str) {
        let parent = zip_path.rsplit_once('/').map(|(p, _)| p.to_string());
        match crate::zipfs::ZipBackend::open(zip_path) {
            Ok(be) => {
                let name = zip_path.rsplit('/').next().unwrap_or("Archiv").to_string();
                self.remote = Some(crate::connect::RemoteState {
                    backend: Arc::new(be),
                    label: format!("📦 {}", name),
                    agent_version: None,
                    zip_return: parent,
                    sftp: None,
                    account: None,
                    endpoint_prefix: None,
                });
                self.start_scan(PathBuf::from("/")); // root inside the archive
            }
            Err(e) => self.error_msg = Some(format!("ZIP konnte nicht geöffnet werden: {e}")),
        }
    }

    /// Extract a local `.zip` into a sibling folder named after the archive,
    /// off-thread; the result + a refresh land via `drain_remote_op`.
    #[cfg(windows)]
    pub(in crate::app) fn start_zip_extract(&mut self, zip_path: String) {
        if self.remote_op_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits eine Aktion…".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let name = zip_path.rsplit('/').next().unwrap_or("archiv").to_string();
        let stem = if name.to_ascii_lowercase().ends_with(".zip") {
            &name[..name.len() - 4]
        } else {
            &name
        };
        let dest = match zip_path.rsplit_once('/') {
            Some((p, _)) => format!("{}/{}", p, stem),
            None => stem.to_string(),
        };
        let dest_pb = PathBuf::from(dest.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("zip-extract".into())
            .spawn(move || {
                let r = crate::zipfs::extract_all(&zip_path, &dest_pb)
                    .map(|n| format!("✓ Entpackt: {} Datei(en)", n))
                    .map_err(|e| format!("Entpacken: {e}"));
                let _ = tx.send(r);
            });
        match spawn {
            Ok(_) => {
                self.remote_op_rx = Some(rx);
                self.notice = Some((format!("📦 Entpacke „{name}“…"), std::time::Instant::now()));
            }
            Err(error) => {
                self.remote_op_rx = None;
                self.error_msg = Some(format!("Entpacken konnte nicht gestartet werden: {error}"));
            }
        }
    }
}
