//! Restore only the recorded object, using two already durable held slots.
use super::{
    platform::{self, Location},
    record::{Record, RestoreOutcome},
    store::Store,
};
use crate::local_access::DirectoryHandle;
use std::{ffi::OsStr, io};

pub(crate) fn restore(id: &str) -> io::Result<RestoreOutcome> {
    let store = Store::open()?;
    restore_in(&store, &store.load(id)?)
}
pub(super) fn restore_in(_store: &Store, record: &Record) -> io::Result<RestoreOutcome> {
    let held = match platform::locate(record)? {
        Location::Held(held) => held,
        Location::Original => {
            platform::verify_original(record)?;
            return Ok(RestoreOutcome::AlreadyAtOriginal);
        }
        Location::Missing => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Aufbewahrter Inhalt wurde nicht gefunden",
            ))
        }
    };
    platform::verify(&held.file, record)?;
    let slots = record.slots();
    let next = DirectoryHandle::checked_quarantine_slot(OsStr::new(slots[1 - held.slot]))?;
    let mut captured =
        held.parent
            .quarantine_regular_child_in(OsStr::new(slots[held.slot]), &held.file, &next)?;
    // The pre-capture immutable record maps both sides of this restore hop.
    // A crash here therefore never creates an unrecorded quarantine.
    let result = platform::verify(captured.file(), record)
        .and_then(|()| captured.move_to(&held.parent, &held.original));
    if let Err(error) = result {
        if let Err(rollback) = captured.restore() {
            return Err(io::Error::new(error.kind(), format!(
                "Wiederherstellen fehlgeschlagen: {error}; Rückstellen fehlgeschlagen: {rollback}; Eintrag {} bleibt erhalten", record.id)));
        }
        return Err(io::Error::new(
            error.kind(),
            format!(
                "Wiederherstellen fehlgeschlagen: {error}; Eintrag {} bleibt im Papierkorb",
                record.id
            ),
        ));
    }
    Ok(RestoreOutcome::Restored)
}
