//! Incremental root indexing reuses the existing safe walker without replacing
//! the persistent complete index with a temporary subset.
use crate::folder_index::{FolderIndex, IndexMsg};
use crossbeam_channel::Receiver;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start_index_delta(
    roots: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
) -> Result<Receiver<IndexMsg>, String> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (out, result) = crossbeam_channel::bounded(1);
    // Cleanup belongs to the worker even when the app closes or drops the result.
    std::thread::Builder::new()
        .name("drive-index-delta".into())
        .spawn(move || {
            let path = crate::app::folder_index_path().with_extension(format!(
                "drive-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            if let Err(error) = FolderIndex::build_async(roots, path.clone(), tx, cancel.clone()) {
                let _ = out.send(IndexMsg::Failed(error.to_string()));
                return;
            }
            while let Ok(message) = rx.recv() {
                if matches!(message, IndexMsg::Progress { .. }) {
                    continue;
                }
                let _ = std::fs::remove_file(&path);
                let _ = out.send(message);
                return;
            }
            let _ = std::fs::remove_file(&path);
        })
        .map_err(|error| error.to_string())?;
    Ok(result)
}
