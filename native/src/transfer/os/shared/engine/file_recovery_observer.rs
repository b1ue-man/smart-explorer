//! Local move quarantine and held-handle publication callbacks.
use std::io;

use super::{
    active, after, after_checked, failure, observe, preparation_failed, previous_target, revoke,
    snapshot, Engine, FileWork, Path, Side, SourceGuard,
};

pub(in crate::transfer::engine) struct LocalObserver<'e, 'a> {
    engine: &'e Engine<'a>,
    file: &'e FileWork,
    source: SourceGuard,
}

impl<'e, 'a> LocalObserver<'e, 'a> {
    pub fn new(engine: &'e Engine<'a>, file: &'e FileWork) -> Self {
        Self {
            engine,
            file,
            source: snapshot(engine.view.source, &file.source),
        }
    }
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
        let source = crate::copy::native_path_text(source)?;
        let mut proof = observe(Side::Local, &source, self.engine.stop_flag())?;
        proof.path = crate::copy::native_path_text(quarantine)?;
        let destination = self.engine.folders.path_of(&self.file.rel);
        record.start_file(
            self.file,
            self.source.clone(),
            None,
            destination.clone(),
            Some(proof),
            previous_target(
                self.engine.view.target,
                &destination,
                self.engine.stop_flag(),
            )?,
        )?;
        active(self.engine)
    }
    fn before_publish(&self, staged: &Path, target: &Path) -> io::Result<()> {
        let Some(record) = &self.engine.job_record else {
            return Ok(());
        };
        active(self.engine)?;
        if !self.source.verified || self.source.directory || self.source.link {
            return Err(failure("Lokale Quelle des Dateibelegs ist unbestätigt"));
        }
        let staged = crate::copy::native_path_text(staged)?;
        let target = crate::copy::native_path_text(target)?;
        let proof = observe(Side::Local, &staged, self.engine.stop_flag())?;
        let previous = previous_target(self.engine.view.target, &target, self.engine.stop_flag())?;
        record.start_file(
            self.file,
            self.source.clone(),
            Some(proof),
            target,
            None,
            previous,
        )?;
        active(self.engine)
    }
    fn after_publish(&self, target: &Path) -> io::Result<()> {
        after(
            self.engine,
            self.file,
            &crate::copy::native_path_text(target)?,
        )
    }
    fn after_publish_handle(&self, target: &Path, held: &std::fs::File) -> io::Result<()> {
        after_checked(
            self.engine,
            self.file,
            &crate::copy::native_path_text(target)?,
            Some(held),
        )
    }
}
