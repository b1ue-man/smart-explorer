//! Integrity guard (plan S1, K3): a session that runs commands for an agent
//! takes a snapshot of the store and checks it after every command. A
//! change that se itself made (the file opens with the key, its generation
//! is not older, `state.json` matches it) is adopted; anything else (file
//! deleted, replaced, older copy, `state.json` edited) is restored from the
//! snapshot and reported, so the caller can lock the session.
use super::store::{read_bounded, MaskStore, SEALED_FILE, STATE_FILE};
use crate::masks::core::doc::{MAX_SEALED_BYTES, MAX_STATE_BYTES};
use crate::masks::core::model::{MaskError, Tampered};

/// Snapshot of the store files at a known good point.
pub struct IntegrityGuard {
    store: MaskStore,
    sealed: Option<Vec<u8>>,
    state: Option<Vec<u8>>,
    config_generation: u64,
}

impl std::fmt::Debug for IntegrityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntegrityGuard")
            .field("store", &self.store)
            .field("config_generation", &self.config_generation)
            .finish_non_exhaustive()
    }
}

impl MaskStore {
    /// A guard over the current, readable store (an unreadable store is
    /// refused: there is nothing good to restore).
    pub fn guard(&self) -> Result<IntegrityGuard, MaskError> {
        let _lock = self.write_guard()?;
        let doc = self.load()?;
        if let Some(doc) = &doc {
            let expected = doc.state().encode();
            let current = read_bounded(&self.path(STATE_FILE), MAX_STATE_BYTES)
                .ok()
                .flatten();
            if current.as_deref() != Some(expected.as_slice()) {
                self.write_side_and_state(doc)?;
            }
        }
        let sealed = read_bounded(&self.path(SEALED_FILE), MAX_SEALED_BYTES)
            .map_err(|e| MaskError::Unreadable(e.to_string()))?;
        let state = read_bounded(&self.path(STATE_FILE), MAX_STATE_BYTES)
            .map_err(|e| MaskError::Unreadable(e.to_string()))?;
        Ok(IntegrityGuard {
            store: self.clone(),
            sealed,
            state,
            config_generation: doc.map_or(0, |doc| doc.config_generation),
        })
    }
}

impl IntegrityGuard {
    /// Checks the store; restores it and reports when it was changed from
    /// outside se.
    pub fn check_and_restore(&mut self) -> Result<(), Tampered> {
        let lock = self.store.write_guard();
        let sealed = read_bounded(&self.store.path(SEALED_FILE), MAX_SEALED_BYTES);
        let state = read_bounded(&self.store.path(STATE_FILE), MAX_STATE_BYTES);
        if let (Ok(sealed), Ok(state)) = (&sealed, &state) {
            if *sealed == self.sealed && *state == self.state && lock.is_ok() {
                return Ok(());
            }
        }
        let what = match (lock.is_ok(), self.store.load()) {
            (true, Ok(Some(doc))) if doc.config_generation >= self.config_generation => {
                let expected = doc.state().encode();
                let state_ok = matches!(&state, Ok(Some(bytes)) if *bytes == expected);
                let adopted = sealed.ok().flatten();
                if state_ok {
                    self.sealed = adopted;
                    self.state = Some(expected);
                    self.config_generation = doc.config_generation;
                    return Ok(());
                }
                let result = self.store.write_side_and_state(&doc);
                self.sealed = adopted;
                self.state = Some(expected);
                self.config_generation = doc.config_generation;
                return Err(Tampered {
                    what: with_result(format!("{STATE_FILE} verändert"), result),
                });
            }
            (true, Ok(None)) if self.sealed.is_none() => {
                // Still no store; a foreign state.json alone neither lifts
                // nor sets a lockdown that a store would have.
                if matches!(&state, Ok(None)) {
                    return Ok(());
                }
                let result = remove_if_present(&self.store, STATE_FILE);
                return Err(Tampered {
                    what: with_result(format!("fremde {STATE_FILE}"), result),
                });
            }
            (false, _) => "Sperrdatei nicht nutzbar".to_string(),
            (_, Ok(None)) => format!("{SEALED_FILE} gelöscht"),
            (_, Ok(Some(_))) => format!("{SEALED_FILE} durch einen älteren Stand ersetzt"),
            (_, Err(error)) => format!("{SEALED_FILE} nicht lesbar ({error})"),
        };
        Err(Tampered {
            what: with_result(what, self.restore()),
        })
    }

    fn restore(&self) -> Result<(), MaskError> {
        // Without a snapshot of the sealed file there is nothing to put
        // back; an unreadable file that appeared stays and keeps the store
        // failing closed instead of deleting what might be the user's lists.
        if let Some(bytes) = &self.sealed {
            self.store.write(SEALED_FILE, bytes)?;
        }
        match &self.state {
            Some(bytes) => self.store.write(STATE_FILE, bytes)?,
            None => remove_if_present(&self.store, STATE_FILE)?,
        }
        if let Some(doc) = self.store.load()? {
            self.store.write_side_and_state(&doc)?;
        }
        self.store.invalidate();
        Ok(())
    }
}

fn remove_if_present(store: &MaskStore, name: &str) -> Result<(), MaskError> {
    match std::fs::remove_file(store.path(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(MaskError::Io(format!("{name} entfernen: {error}"))),
    }
}

fn with_result(what: String, result: Result<(), MaskError>) -> String {
    match result {
        Ok(()) => what,
        Err(error) => format!("{what}; Wiederherstellung fehlgeschlagen: {error}"),
    }
}
