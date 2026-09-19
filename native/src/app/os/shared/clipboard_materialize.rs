//! Windows virtual descriptors have a fixed MAX_PATH-sized relative name.
//! Longer trees use the same exact file snapshot in a temporary directory.
use super::clipboard_lifecycle::PreparedTempClipboard;
use super::clipboard_state::{PreparationResult, PreparationStamp};
use super::prelude::*;
use super::*;
use crate::app::shared_platform_helpers::ClipboardVirtualFile;

impl App {
    pub(in crate::app) fn materialize_long_clipboard(
        &mut self,
        files: Vec<ClipboardVirtualFile>,
        stamp: PreparationStamp,
    ) {
        let (tx, rx) = unbounded();
        let worker = std::thread::Builder::new()
            .name("clip-long-paths".into())
            .spawn(move || {
                let backend = crate::vfs::LocalBackend::new("");
                let result =
                    download_clipboard_snapshot(&backend, files).map(PreparedTempClipboard::new);
                let _ = tx.send(PreparationResult { stamp, result });
            });
        match worker {
            Ok(_) => {
                self.clip_download_rx = Some(rx);
                self.notice = Some((
                    "Bereite Dateien mit langen relativen Pfaden für die Zwischenablage vor …"
                        .into(),
                    Instant::now(),
                ));
            }
            Err(error) => {
                self.cancel_clipboard_preparation();
                self.error_msg = Some(format!(
                    "Zwischenablage-Vorbereitung konnte nicht starten: {error}"
                ));
            }
        }
    }
}
