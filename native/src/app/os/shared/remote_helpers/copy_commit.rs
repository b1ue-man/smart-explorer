//! Explicit create/replace commit policy and private remote staging.
use crate::vfs::Backend;
use std::io::Write;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy)]
pub(super) enum CommitMode { Create, Replace }

pub(super) struct StagedUpload<'a> {
    backend: &'a dyn Backend,
    destination: String,
    path: String,
    writer: Box<dyn Write + Send>,
}

impl<'a> StagedUpload<'a> {
    pub(super) fn open(
        backend: &'a dyn Backend, destination: &str, cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        super::cancel::check_optional(cancel)?;
        if let Some((parent, _)) = destination.rsplit_once('/') {
            backend.mkdir_all(parent).map_err(|error| format!("Zielordner „{parent}“ anlegen: {error}"))?;
            super::cancel::check_optional(cancel)?;
        }
        let path = crate::vfs::unique_staging_path(backend, destination, "upload")
            .map_err(|error| format!("Upload-Stufe für „{destination}“ reservieren: {error}"))?;
        super::cancel::check_optional(cancel)?;
        // Attempt the actual primitive, not a potentially expensive capability
        // probe. Unsupported must remain explicit; never fall back to truncate.
        let writer = backend.open_write_copy_stage(&path).map_err(|error| {
            format!("Private Upload-Stufe „{path}“ öffnen ({:?}): {error}; kein unsicherer Schreib-Fallback", error.kind())
        })?;
        Ok(Self { backend, destination: destination.to_string(), path, writer })
    }

    pub(super) fn writer(&mut self) -> &mut dyn Write { self.writer.as_mut() }

    pub(super) fn failed(self, error: String) -> String {
        let Self { path, writer, .. } = self;
        drop(writer);
        retained_stage_error(&path, error)
    }

    pub(super) fn commit(
        self, mode: CommitMode, cancel: Option<&AtomicBool>,
        verify_source: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let Self { backend, destination, path, mut writer } = self;
        let flushed = super::cancel::check_optional(cancel)
            .and_then(|_| writer.flush().map_err(|error| format!("Upload bestätigen ({:?}): {error}", error.kind())));
        drop(writer);
        flushed.map_err(|error| retained_stage_error(&path, error))?;
        verify_source().and_then(|_| super::cancel::check_optional(cancel))
            .map_err(|error| retained_stage_error(&path, error))?;
        let promoted = match mode {
            CommitMode::Create => backend.promote_copy_stage(&path, &destination),
            CommitMode::Replace => crate::vfs::promote_staged_replace(backend, &path, &destination),
        };
        promoted.map_err(|error| retained_stage_error(&path,
            format!("Upload-Ziel „{destination}“ veröffentlichen ({:?}): {error}", error.kind())))?;
        // Acknowledged publication is success even if cancellation arrived
        // during promotion. The caller counts it before stopping later work.
        Ok(())
    }
}

fn retained_stage_error(path: &str, error: String) -> String {
    // Some providers create only at flush; even a previously created pathname
    // can be exchanged. A failed operation is not proof that remove_file would
    // still target our object. Preserve the source and disclose the exact stage.
    format!("{error}; mögliche Upload-Stufe „{path}“ wurde nicht automatisch gelöscht; Quelldatei bleibt erhalten")
}
