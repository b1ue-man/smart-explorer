use super::prelude::*;
use super::*;

enum UploadSelection {
    Paths(Vec<String>),
    Filtered(Vec<(String, String)>),
}

impl App {
    /// Upload local files/folders. Shared by clipboard and drag/drop routing.
    pub(in crate::app) fn start_remote_upload(
        &mut self,
        paths: Vec<String>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        self.start_clipboard_upload(UploadSelection::Paths(paths), backend, dest_root);
    }

    /// Preserve the filtered clipboard's relative paths rather than flattening
    /// each absolute source path into the destination directory.
    pub(in crate::app) fn start_filtered_remote_upload(
        &mut self,
        pairs: Vec<(String, String)>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        self.start_clipboard_upload(UploadSelection::Filtered(pairs), backend, dest_root);
    }

    fn start_clipboard_upload(
        &mut self,
        selection: UploadSelection,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Upload — bitte warten.".to_string(),
                Instant::now(),
            ));
            return;
        }
        let n = match &selection {
            UploadSelection::Paths(paths) => paths.len(),
            UploadSelection::Filtered(pairs) => pairs.len(),
        };
        let (tx, rx) = unbounded();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let spawn = std::thread::Builder::new()
            .name("remote-upload".into())
            .spawn(move || match selection {
                UploadSelection::Paths(paths) => {
                    upload_paths_progress(&*backend, &paths, &dest_root, &tx, &worker_cancel);
                }
                UploadSelection::Filtered(pairs) => {
                    upload_pairs_progress(&*backend, &pairs, &dest_root, &tx, &worker_cancel);
                }
            });
        match spawn {
            Ok(worker) => {
                self.upload_rx = Some(rx);
                self.transfer_cancel = Some(cancel);
                self.transfer_worker = Some(worker);
                self.transfer_progress = Some(TransferProgress::new(
                    TransferKind::Upload, "Lade hoch", n as u64, 0,
                ));
                self.notice = Some((format!("⬆ Lade {n} Element(e) hoch…"), Instant::now()));
            }
            Err(error) => {
                self.upload_rx = None;
                self.transfer_progress = None;
                self.transfer_cancel = None;
                self.transfer_worker = None;
                self.error_msg = Some(format!("Remote-Upload konnte nicht gestartet werden: {error}"));
            }
        }
    }
}
