//! The publication hooks of one local packet member (M3, R4): at the name
//! the packet's intent covers (which records the member's stage) it needs
//! no intent of its own (only the job's stop is checked) and is acknowledged
//! through its still open stage handle; a failed rename there is revoked
//! against that stage. At any other name (a numbered one) it is the single
//! file's observer (`LocalObserver`), which records its own intent with the
//! stage first.
use super::super::super::file_recovery::{self, LocalObserver};
use super::super::super::queue::FileWork;
use super::super::super::Engine;
use crate::copy::PublicationObserver;
use std::cell::OnceCell;
use std::io;
use std::path::Path;

/// The hooks of one member; `covered` is the name the packet's intent
/// names, `None` for a member that was numbered before it.
pub(super) struct PacketObserver<'e, 'a> {
    engine: &'e Engine<'a>,
    file: &'e FileWork,
    covered: Option<&'e Path>,
    single: OnceCell<LocalObserver<'e, 'a>>,
}

impl<'e, 'a> PacketObserver<'e, 'a> {
    pub(super) fn new(
        engine: &'e Engine<'a>,
        file: &'e FileWork,
        covered: Option<&'e Path>,
    ) -> Self {
        Self {
            engine,
            file,
            covered,
            single: OnceCell::new(),
        }
    }

    fn covers(&self, target: &Path) -> bool {
        self.covered == Some(target)
    }

    fn single(&self) -> &LocalObserver<'e, 'a> {
        self.single
            .get_or_init(|| LocalObserver::new(self.engine, self.file))
    }

    /// The packet's intent is durable; publishing waits only for the stop.
    fn active(&self) -> io::Result<()> {
        if self.engine.stopped() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                crate::transfer::cancel::CANCELED_ERROR,
            ));
        }
        Ok(())
    }

    fn acknowledge(&self, target: &Path, held: Option<&std::fs::File>) -> io::Result<()> {
        #[cfg(test)]
        if super::tests::ack_lost() {
            return Err(io::Error::other("crash before the acknowledgement (test)"));
        }
        let destination = crate::copy::native_path_text(target)?;
        match held {
            Some(held) => file_recovery::after_held(self.engine, self.file, &destination, held),
            None => file_recovery::after(self.engine, self.file, &destination),
        }
    }
}

impl PublicationObserver for PacketObserver<'_, '_> {
    fn before_publish(&self, staged: &Path, target: &Path) -> io::Result<()> {
        if self.covers(target) {
            return self.active();
        }
        self.single().before_publish(staged, target)
    }
    fn before_publish_handle(
        &self,
        staged: &Path,
        target: &Path,
        file: &std::fs::File,
    ) -> io::Result<()> {
        if self.covers(target) {
            return self.active();
        }
        self.single().before_publish_handle(staged, target, file)
    }
    fn after_publish(&self, target: &Path) -> io::Result<()> {
        if self.covers(target) {
            return self.acknowledge(target, None);
        }
        self.single().after_publish(target)
    }
    fn after_publish_handle(&self, target: &Path, held: &std::fs::File) -> io::Result<()> {
        if self.covers(target) {
            return self.acknowledge(target, Some(held));
        }
        self.single().after_publish_handle(target, held)
    }
    fn publication_preparation_failed(&self, _: &Path, _: &Path) -> io::Result<()> {
        file_recovery::preparation_failed(self.engine, self.file)
    }
    /// A failed rename is revoked against the receipt's stage and name,
    /// covered or not: durably not authorized before the stage goes or a
    /// numbered name gets its own intent.
    fn publication_failed(&self, staged: &Path, target: &Path) -> io::Result<()> {
        self.single().publication_failed(staged, target)
    }
}
