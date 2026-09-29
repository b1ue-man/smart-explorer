use super::copy_commit::{CommitMode, StagedUpload};
use super::engine::{run_legacy, Side};
use super::job::JobItems;
use super::types::TransferMsg;
use super::upload_stream::UploadSource;
use std::path::Path;
use std::sync::atomic::AtomicBool;

fn upload_with_mode(
    backend: &dyn crate::vfs::Backend,
    source: &Path,
    destination: &str,
    mode: CommitMode,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    super::cancel::check_optional(cancel)?;
    let mut source = UploadSource::open(source)?;
    let mut staged = StagedUpload::open(backend, destination, cancel)?;
    if let Err(error) = source.copy_to(staged.writer(), cancel, progress) {
        return Err(staged.failed(error));
    }
    staged.commit(mode, cancel, || source.verify())
}

/// Explicit save-back/overwrite operation. Clipboard copies use the engine.
pub fn upload_file(
    backend: &dyn crate::vfs::Backend,
    source: &Path,
    destination: &str,
) -> Result<(), String> {
    upload_with_mode(
        backend,
        source,
        destination,
        CommitMode::Replace,
        None,
        |_| {},
    )
}

/// Uploads local entries (files and whole folders) into `destination`
/// through the streaming engine: numbered names for taken top-level names,
/// create-only publication, never a replacement.
pub fn upload_paths_progress(
    backend: &dyn crate::vfs::Backend,
    paths: &[String],
    destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    run_legacy(
        Side::Local,
        Side::Remote(backend),
        destination,
        JobItems::Roots {
            paths: paths.to_vec(),
            base: None,
        },
        None,
        tx,
        cancel,
    );
}
