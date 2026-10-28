//! Local move quarantine and held-handle publication callbacks.
//!
//! The source snapshot is taken only for a job with a crash log (M5,
//! 2026-10-28): without one no hook records anything, and the copy checks
//! the source itself (`unchanged_state`). The stage's evidence comes from
//! its open handle, which the copy has just compared with the stage path
//! (M4): no path reopening, no second identity look.
use std::fs::File;
use std::io;

use super::super::file_evidence::{stamp, Proof};
use super::{
    active, after, after_checked, failure, preparation_failed, previous_presence, revoke, snapshot,
    Engine, FileWork, Path, Side, SourceGuard,
};

pub(in crate::transfer::engine) struct LocalObserver<'e, 'a> {
    engine: &'e Engine<'a>,
    file: &'e FileWork,
    /// The source as it was before the copy; `None` without a crash log.
    source: Option<SourceGuard>,
}

impl<'e, 'a> LocalObserver<'e, 'a> {
    pub fn new(engine: &'e Engine<'a>, file: &'e FileWork) -> Self {
        let source = engine
            .job_record
            .as_ref()
            .map(|_| snapshot(engine.view.source, &file.source));
        Self {
            engine,
            file,
            source,
        }
    }

    /// The recorded source of a journaled job.
    fn source(&self) -> io::Result<&SourceGuard> {
        self.source
            .as_ref()
            .ok_or_else(|| failure("Lokale Quelle des Dateibelegs fehlt"))
    }

    /// The durable intent of publishing the stage `staged` (evidence
    /// `proof`) at `target`.
    fn intend(&self, target: &Path, proof: Proof) -> io::Result<()> {
        let Some(record) = &self.engine.job_record else {
            return Ok(());
        };
        let source = self.source()?;
        if !source.verified || source.directory || source.link {
            return Err(failure("Lokale Quelle des Dateibelegs ist unbestätigt"));
        }
        let target = crate::copy::native_path_text(target)?;
        let previous = previous_presence(self.engine.view.target, &target)?;
        record.start_file(
            self.file,
            source.clone(),
            Some(proof),
            target,
            None,
            previous,
        )?;
        active(self.engine)
    }
}

/// Modification time in milliseconds as the local backend reports it.
fn mtime_ms(metadata: &std::fs::Metadata) -> i64 {
    match metadata.modified() {
        Ok(time) => match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => after.as_millis() as i64,
            Err(before) => -(before.duration().as_millis() as i64),
        },
        Err(_) => 0,
    }
}

/// `stamp` of the local file behind the open `file` (named `path`): the same
/// size, time and object identity a stat and identity look at the path
/// report, read from the handle. `object` is the identity when the caller
/// already has it.
fn handle_stamp(path: &str, file: &File, object: Option<Option<String>>) -> io::Result<Proof> {
    let metadata = file.metadata()?;
    let class = crate::local_access::classify_open_file(file)?;
    if !metadata.is_file() || class.link_like || class.special {
        return Err(io::Error::other(format!(
            "Dateibeleg verweigert Ordner, Link oder Spezialdatei: {path}"
        )));
    }
    let object = match object {
        Some(object) => object,
        None => match crate::copy::local_file_identity_handle(file) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
            Err(error) => return Err(error),
        },
    };
    Ok(Proof {
        path: path.to_string(),
        object,
        id: None,
        size: metadata.len(),
        mtime_ms: mtime_ms(&metadata),
        sha256: String::new(),
    })
}

/// `file_evidence::confirm_stamp` for a just published local file whose
/// handle is still open (M4): the published name must name the held object
/// (one look at the path) and the handle must still show the recorded size
/// and time. Returns the target's stamp.
pub(super) fn confirm_held(proof: &Proof, file: &File) -> io::Result<Proof> {
    let changed = || {
        io::Error::other(format!(
            "Datei ist geändert oder durch ein fremdes Objekt ersetzt: {}",
            proof.path
        ))
    };
    if proof.object.is_none() {
        return Err(changed());
    }
    let path = super::super::super::walk_listers::native(&proof.path);
    if !crate::copy::path_names_handle(&path, file)? {
        return Err(changed());
    }
    // The held handle is the recorded object: its identity stays.
    let now = handle_stamp(&proof.path, file, Some(proof.object.clone()))?;
    if now.size != proof.size || now.mtime_ms != proof.mtime_ms {
        return Err(changed());
    }
    Ok(now)
}

impl crate::copy::PublicationObserver for LocalObserver<'_, '_> {
    fn publication_preparation_failed(&self, _: &Path, _: &Path) -> io::Result<()> {
        preparation_failed(self.engine, self.file)
    }
    fn publication_failed(&self, staged: &Path, target: &Path) -> io::Result<()> {
        revoke(self.engine, self.file, Some((staged, target)))
    }
    fn before_source_quarantine(&self, source: &Path, quarantine: &Path) -> io::Result<()> {
        let Some(record) = &self.engine.job_record else {
            return Ok(());
        };
        active(self.engine)?;
        // The quarantined source by metadata (a rename keeps its object);
        // nothing is read.
        let source = crate::copy::native_path_text(source)?;
        let mut proof = stamp(Side::Local, &source)?;
        proof.path = crate::copy::native_path_text(quarantine)?;
        let destination = self.engine.folders.path_of(&self.file.rel);
        record.start_file(
            self.file,
            self.source()?.clone(),
            None,
            destination.clone(),
            Some(proof),
            previous_presence(self.engine.view.target, &destination)?,
        )?;
        active(self.engine)
    }
    fn before_publish(&self, staged: &Path, target: &Path) -> io::Result<()> {
        if self.engine.job_record.is_none() {
            return Ok(());
        }
        active(self.engine)?;
        // The stage's metadata identity becomes the publication evidence:
        // the rename keeps the object. Nothing is read (2026-10-27).
        let proof = stamp(Side::Local, &crate::copy::native_path_text(staged)?)?;
        self.intend(target, proof)
    }
    fn before_publish_handle(&self, staged: &Path, target: &Path, file: &File) -> io::Result<()> {
        if self.engine.job_record.is_none() {
            return Ok(());
        }
        active(self.engine)?;
        let proof = handle_stamp(&crate::copy::native_path_text(staged)?, file, None)?;
        self.intend(target, proof)
    }
    fn after_publish(&self, target: &Path) -> io::Result<()> {
        after(
            self.engine,
            self.file,
            &crate::copy::native_path_text(target)?,
        )
    }
    fn after_publish_handle(&self, target: &Path, held: &File) -> io::Result<()> {
        after_checked(
            self.engine,
            self.file,
            &crate::copy::native_path_text(target)?,
            Some(held),
            None,
        )
    }
}
