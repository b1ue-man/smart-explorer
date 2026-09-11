//! Bounded upload reads and validation of the opened local source observation.
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

#[derive(PartialEq, Eq)]
struct Observation {
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

fn observation(metadata: &Metadata, path: &Path) -> Result<Observation, String> {
    if super::super::upload_is_link_like(metadata) || !metadata.is_file() {
        return Err(format!("{}: Upload-Quelle ist keine reguläre Datei ohne Link/Reparse-Punkt", path.display()));
    }
    Ok(Observation {
        length: metadata.len(),
        modified: metadata.modified().map_err(|error| format!("{}: Änderungszeit lesen: {error}", path.display()))?,
        created: metadata.created().ok(),
    })
}

pub(super) struct UploadSource {
    file: File,
    path: PathBuf,
    before: Observation,
}

impl UploadSource {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        let before = observation(&std::fs::symlink_metadata(path)
            .map_err(|error| format!("{}: Quelle prüfen: {error}", path.display()))?, path)?;
        let file = File::open(path).map_err(|error| format!("{}: Quelle öffnen: {error}", path.display()))?;
        let source = Self { file, path: path.to_path_buf(), before };
        source.verify()?;
        Ok(source)
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        let opened = observation(&self.file.metadata()
            .map_err(|error| format!("{}: Geöffnete Quelle prüfen: {error}", self.path.display()))?, &self.path)?;
        let named = observation(&std::fs::symlink_metadata(&self.path)
            .map_err(|error| format!("{}: Quellpfad erneut prüfen: {error}", self.path.display()))?, &self.path)?;
        if opened != self.before || named != self.before {
            return Err(format!("{}: Quelle wurde während der Übertragung geändert", self.path.display()));
        }
        Ok(())
    }

    pub(super) fn copy_to(
        &mut self, writer: &mut dyn Write, cancel: Option<&AtomicBool>,
        mut progress: impl FnMut(u64),
    ) -> Result<(), String> {
        let expected = self.before.length;
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            super::cancel::check_optional(cancel)?;
            // At most one byte beyond the opened length: a growing file cannot
            // turn this copy into an unbounded upload or be silently accepted.
            let amount = expected.saturating_sub(copied).saturating_add(1)
                .min(buffer.len() as u64) as usize;
            let read = self.file.read(&mut buffer[..amount])
                .map_err(|error| format!("{}: Quelle lesen: {error}", self.path.display()))?;
            super::cancel::check_optional(cancel)?;
            if read == 0 { break; }
            if read as u64 > expected.saturating_sub(copied) {
                return Err(format!("{}: Quelle ist während der Übertragung gewachsen", self.path.display()));
            }
            writer.write_all(&buffer[..read]).map_err(|error| format!("Upload schreiben: {error}"))?;
            copied += read as u64;
            progress(read as u64);
        }
        if copied != expected {
            return Err(format!("{}: Quelle unvollständig gelesen ({copied} von {expected} Bytes)", self.path.display()));
        }
        self.verify()
    }
}
