//! Who may change what (plan S2 and the contract): every actor may create a
//! list; strengthening changes (add entries or scopes, enable, lock, raise a
//! tolerance) are refused to the agent on a locked list; weakening changes
//! (unlock, disable, remove, delete, change a value, lower a tolerance) need
//! `VerifiedUser`; clear text only reaches `Gui`.
use super::doc::{EntryDoc, ListDoc};
use super::fold::fold_pattern;
use super::model::{Actor, EntryKind, EntryUpdate, MaskError};

/// Direction of a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Strengthen,
    Weaken,
}

pub(crate) fn check(actor: Actor, list: &ListDoc, change: Change) -> Result<(), MaskError> {
    match change {
        Change::Weaken if actor != Actor::VerifiedUser => Err(MaskError::NotAllowed(
            "die Änderung schwächt den Schutz und braucht die Bestätigung des Nutzers".into(),
        )),
        Change::Strengthen if actor == Actor::Agent && list.locked => {
            Err(MaskError::Locked(list.name.clone()))
        }
        _ => Ok(()),
    }
}

pub(crate) fn check_reveal(actor: Actor) -> Result<(), MaskError> {
    if actor == Actor::Gui {
        Ok(())
    } else {
        Err(MaskError::NotAllowed(
            "Klartext der Einträge zeigt nur die Desktop-Oberfläche".into(),
        ))
    }
}

/// Classifies an entry update: it strengthens only when the value stays
/// and every flag finds at least as much as before.
pub(crate) fn classify_update(entry: &EntryDoc, update: &EntryUpdate) -> Change {
    if update
        .value
        .as_ref()
        .is_some_and(|value| *value != entry.value)
    {
        return Change::Weaken;
    }
    if update.case_sensitive == Some(true) && !entry.case_sensitive {
        return Change::Weaken;
    }
    if update.whole_word == Some(true) && !entry.whole_word {
        return Change::Weaken;
    }
    if let Some(tolerance) = update.tolerance {
        if entry.kind == EntryKind::Fuzzy {
            let len = fold_pattern(&entry.value).len();
            if !tolerance.covers(entry.tolerance, len) {
                return Change::Weaken;
            }
        }
    }
    Change::Strengthen
}
