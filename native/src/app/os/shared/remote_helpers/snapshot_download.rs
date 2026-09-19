use super::download_file::download_file_progress;
use super::{cleanup_temp_copy, open_temp_path};
use crate::app::app_models::{TransferKind, TransferProgress};
use crate::app::shared_platform_helpers::ClipboardVirtualFile;

/// Materialize exactly the selected files, with a shared relative root. The
/// clipboard receives its children, never the implementation's wrapper folder.
pub(in crate::app) fn download_clipboard_snapshot(
    backend: &dyn crate::vfs::Backend,
    files: Vec<ClipboardVirtualFile>,
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let root = open_temp_path("Auswahl").map_err(|error| error.to_string())?;
    let result = (|| {
        std::fs::create_dir(&root).map_err(|error| error.to_string())?;
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let bytes = files.iter().fold(0u64, |sum, file| sum.saturating_add(file.size));
        let mut progress = TransferProgress::new(TransferKind::Download, "Auswahl vorbereiten", files.len() as u64, bytes);
        let mut last = std::time::Instant::now();
        for file in files {
            // Do not traverse again: the snapshot already captures membership.
            // Revalidate names at the write boundary even for internal callers.
            for component in file.rel.split('/') {
                crate::vfs::validate_child_name(component).map_err(|error| error.to_string())?;
            }
            let dest = root.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            download_file_progress(backend, &file.abs, &dest, file.size,
                &tx, &mut progress, &mut last, None)?;
        }
        std::fs::read_dir(&root).map_err(|error| error.to_string())?
            .map(|entry| entry.map(|entry| entry.path().to_string_lossy().into_owned())
                .map_err(|error| error.to_string()))
            .collect()
    })();
    if result.is_err() {
        cleanup_temp_copy(&root);
    }
    result
}
