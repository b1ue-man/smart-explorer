use super::entries::RemoteFileEntry;
use super::temp::{cleanup_temp_copy, open_temp_path};
use crate::filter::tree::ClipboardVirtualFile;

/// Materialize exactly the selected files, with a shared relative root. The
/// clipboard receives its children, never the implementation's wrapper folder.
pub fn download_clipboard_snapshot(
    backend: &dyn crate::vfs::Backend,
    files: Vec<ClipboardVirtualFile>,
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let root = open_temp_path("Auswahl").map_err(|error| error.to_string())?;
    let result: Result<Vec<String>, String> = (|| {
        std::fs::create_dir(&root).map_err(|error| error.to_string())?;
        let mut entries = Vec::with_capacity(files.len());
        let mut folders = std::collections::BTreeSet::new();
        for file in files {
            // Do not traverse again: the snapshot already captures membership.
            // Revalidate names at the write boundary even for internal callers.
            for component in file.rel.split('/') {
                crate::vfs::validate_child_name(component).map_err(|error| error.to_string())?;
            }
            if let Some((parent, _)) = file.rel.rsplit_once('/') {
                folders.insert(parent.to_string());
            }
            entries.push(RemoteFileEntry {
                src: file.abs,
                rel: file.rel,
                size: file.size,
            });
        }
        // Every folder once (K-H), then the files side by side under the
        // flow of their connection.
        for folder in folders {
            std::fs::create_dir_all(root.join(folder.replace('/', std::path::MAIN_SEPARATOR_STR)))
                .map_err(|error| error.to_string())?;
        }
        super::clipboard_parallel::download_files_parallel(backend, &entries, &root)?;
        std::fs::read_dir(&root)
            .map_err(|error| error.to_string())?
            .map(|entry| {
                entry
                    .map(|entry| entry.path().to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .collect()
    })();
    if result.is_err() {
        cleanup_temp_copy(&root);
    }
    result
}
