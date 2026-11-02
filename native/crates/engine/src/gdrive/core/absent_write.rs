//! The proof behind `open_write_absent` (G3-08b, 2026-10-28): a new file was
//! created under its final name in an existing folder with one write
//! request (sized_writer.rs `Purpose::Absent`). Drive reserves no names, so
//! right after the create the name is listed: it must lead to the new ID
//! alone. Any other answer (a second object of that name, an own object the
//! listing does not show, a failed listing) withdraws the own object to the
//! trash by its exact ID and reports `AlreadyExists`: nothing of this writer
//! stays under the name, and the engine takes its staged path, which numbers
//! the name. Only a failed withdrawal is reported as an open outcome.
//!
//! The same guarantee as the staged path (`promotion.rs`
//! `rename_absent_locked`): unique name after completion, own ID rolled back
//! otherwise. A crash between the create and the proof can leave two
//! objects of one name, as a crash between the staged path's rename and its
//! check can.
use super::new_object::NewObject;
use super::promotion_api::DriveObject;
use super::GDriveBackend;
use std::io;

/// What the listing after the create says about the name.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NameProof {
    /// Exactly the own ID.
    Unique,
    /// The own ID is not (yet) listed and nothing else is: ask once more.
    NotListed,
    /// Another object carries the name too (or instead).
    Taken,
}

/// Decides a listing of the name against the own ID.
pub(super) fn judge(objects: &[DriveObject], own: &str) -> NameProof {
    if objects.iter().any(|object| object.id != own) {
        return NameProof::Taken;
    }
    match objects.len() {
        1 => NameProof::Unique,
        // Drive never lists one ID twice; an empty answer did not see it yet.
        _ => NameProof::NotListed,
    }
}

impl GDriveBackend {
    /// After the create of `object` under its final name: `Ok` when the name
    /// leads to its ID alone, else the own object is trashed and the error
    /// is `AlreadyExists` (nothing published), or another kind when the own
    /// object could not be withdrawn (outcome open, named in the message).
    pub(super) fn settle_absent(&self, object: &NewObject, key: &str) -> io::Result<()> {
        match self.prove_absent(object) {
            Ok(()) => Ok(()),
            Err(reason) => self.withdraw_absent(object, key, &reason),
        }
    }

    /// Lists the name (a second time when the first answer does not show the
    /// new ID yet); `Err` carries why the name is not proven unique.
    fn prove_absent(&self, object: &NewObject) -> Result<(), String> {
        let mut asked = 0;
        loop {
            asked += 1;
            let objects = self
                .named_objects(&object.parent_id, &object.title)
                .map_err(|error| format!("Auflistung des Namens scheiterte: {error}"))?;
            match judge(&objects, &object.id) {
                NameProof::Unique => return Ok(()),
                NameProof::Taken => {
                    return Err(format!("{} Objekte tragen den Namen", objects.len()))
                }
                NameProof::NotListed if asked < 2 => {}
                NameProof::NotListed => {
                    return Err("die Auflistung zeigt das neue Objekt nicht".to_string())
                }
            }
        }
    }

    /// Trashes the own new object by its exact ID and reports the name as
    /// taken; a failed trash is reported with the ID that may remain.
    fn withdraw_absent(&self, object: &NewObject, key: &str, reason: &str) -> io::Result<()> {
        if let Err(error) = self.trash_id(&object.id) {
            return Err(io::Error::new(
                if error.kind() == io::ErrorKind::AlreadyExists {
                    io::ErrorKind::Other
                } else {
                    error.kind()
                },
                format!(
                    "Drive-Name „{}“ ist nicht eindeutig ({reason}); das eigene Objekt {} konnte nicht in den Papierkorb: {error}",
                    object.title, object.id
                ),
            ));
        }
        if let Ok(mut pending) = self.pending_upload_ids_guard() {
            if pending.get(key) == Some(&object.id) {
                pending.remove(key);
            }
        }
        self.forget_path_prefix(key);
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Drive-Name „{}“ ist nicht frei ({reason}); das eigene Objekt {} liegt im Papierkorb, nichts wurde veröffentlicht",
                object.title, object.id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{judge, DriveObject, NameProof};

    fn object(id: &str) -> DriveObject {
        DriveObject {
            id: id.to_string(),
            mime_type: "application/octet-stream".to_string(),
            size: Some(1),
            md5: None,
            mtime_ms: None,
        }
    }

    #[test]
    fn speed_task_drive_absent_name_proof() {
        assert_eq!(judge(&[object("own")], "own"), NameProof::Unique);
        assert_eq!(judge(&[], "own"), NameProof::NotListed);
        let both = [object("own"), object("x")];
        assert_eq!(judge(&both, "own"), NameProof::Taken);
        assert_eq!(judge(&[object("x")], "own"), NameProof::Taken);
    }
}
