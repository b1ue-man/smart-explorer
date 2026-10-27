//! Whether a saved publication took place, decided when a job is resumed:
//! by metadata first, by content digest only in doubt.
use std::io;

use super::{
    doubt, failure, verify, with_backend, AtomicBool, JobView, PreviousTarget, Proof, Receipt, Side,
};

/// What establishes a publication, for a move's source cleanup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Evidence {
    /// The target object to bind; `None` for a remote target (never a move).
    pub(super) identity: Option<String>,
    /// The content the target must hold as well; `None` when its unchanged
    /// metadata identity decided.
    pub(super) sha256: Option<String>,
}

/// `Some` when the receipt's publication took place.
pub(super) fn publication_state(
    receipt: &Receipt,
    view: &JobView<'_>,
    cancel: &AtomicBool,
) -> io::Result<Option<Evidence>> {
    let side = view.target;
    // A pending new-key intent, failed preparation, or live-proven failed
    // publication can write this durable negative acknowledgement.
    if !receipt.publication_authorized {
        return Ok(None);
    }
    let Some(proof) = &receipt.proof else {
        if receipt.quarantine.is_some() || (side.is_local() && stage_present(receipt, side)?) {
            return Ok(None);
        }
        // No stored evidence (a remote publication): a destination of the
        // source's kind and size stays this job's candidate; its content is
        // decided once, in parallel, when the file's turn comes
        // (`missing_retry::resume`). Anything else is not this job's file
        // and is transferred again under the job's conflict policy.
        let candidate = doubt::shape(receipt, view.source, side)?.is_none();
        return Ok(candidate.then_some(Evidence {
            identity: None,
            sha256: None,
        }));
    };
    if proof.sha256.is_empty() {
        return by_metadata(receipt, proof, view, cancel);
    }
    // A content proof of a receipt written by 0.5.185/0.5.186: its digest,
    // recomputed on the target's device, plus the recorded object identity.
    match verify(proof, side, cancel, false) {
        Ok(()) => {
            return Ok(Some(Evidence {
                identity: proof.object.clone(),
                sha256: Some(proof.sha256.clone()),
            }))
        }
        Err(error) if receipt.confirmed || error.kind() == io::ErrorKind::Interrupted => {
            return Err(error)
        }
        Err(_) => {}
    }
    let stage = receipt
        .staged
        .as_ref()
        .ok_or_else(|| failure("Vorpublikationszustand ohne eigene Stage"))?;
    let stage_present = match with_backend(side, |backend| backend.stat(&stage.path)) {
        Ok(_) => {
            verify(stage, side, cancel, true)?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !stage_present || !previous_matches(receipt, side, cancel)? {
        return Err(failure(format!(
            "Zielersatz oder unbestätigte Publikationsphase: {}",
            receipt.destination
        )));
    }
    Ok(None)
}

/// A local publication's evidence is its stage's object, renamed into place.
/// Unchanged metadata decides; a changed target is decided by content
/// against the source (or the quarantined move source).
fn by_metadata(
    receipt: &Receipt,
    proof: &Proof,
    view: &JobView<'_>,
    cancel: &AtomicBool,
) -> io::Result<Option<Evidence>> {
    // A stage still at its private name was never renamed into place.
    if stage_present(receipt, view.target)? {
        return Ok(None);
    }
    match verify(proof, view.target, cancel, false) {
        Ok(()) => {
            return Ok(Some(Evidence {
                identity: proof.object.clone(),
                sha256: None,
            }))
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(_) => {}
    }
    Ok(
        match doubt::content(receipt, view.source, view.target, cancel)? {
            Some((doubt::Ownership::Own, Some(target))) => Some(Evidence {
                identity: target.object,
                sha256: Some(target.sha256),
            }),
            Some(_) => None,
            // Neither the source nor its quarantine is left: a move completed
            // before; nothing remains to recover or to remove.
            None => Some(Evidence {
                identity: proof.object.clone(),
                sha256: None,
            }),
        },
    )
}

fn stage_present(receipt: &Receipt, side: Side<'_>) -> io::Result<bool> {
    let Some(stage) = &receipt.staged else {
        return Ok(false);
    };
    match with_backend(side, |backend| backend.stat(&stage.path)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn previous_matches(
    receipt: &Receipt,
    side: Side<'_>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    match &receipt.previous {
        Some(PreviousTarget::Absent) => {
            match with_backend(side, |backend| backend.stat(&receipt.destination)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
                Err(error) => Err(error),
                Ok(_) => Ok(false),
            }
        }
        Some(PreviousTarget::File(proof)) => match verify(proof, side, cancel, true) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(_) => Ok(false),
        },
        _ => Ok(false),
    }
}
