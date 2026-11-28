//! Management of the lists through the store: every change runs under the
//! write guard on a freshly read document (`core::admin` holds the rules)
//! and is saved before the call returns.
use super::side;
use super::store::{MaskStore, SEALED_FILE, STATE_FILE};
use crate::masks::core::admin;
use crate::masks::core::doc::StoreDoc;
use crate::masks::core::model::{
    Actor, EntryId, EntrySummary, EntryUpdate, EntryWithValue, ListId, ListSummary, MaskError,
    NewEntry,
};

impl MaskStore {
    /// Every list without values.
    pub fn lists(&self) -> Result<Vec<ListSummary>, MaskError> {
        Ok(self
            .load()?
            .map(|doc| admin::summaries(&doc))
            .unwrap_or_default())
    }

    /// The id of the list called `name` (Groß/Klein ignored).
    pub fn find_list(&self, name: &str) -> Result<Option<ListId>, MaskError> {
        let wanted = name.trim().to_lowercase();
        Ok(self
            .lists()?
            .into_iter()
            .find(|list| list.name.to_lowercase() == wanted)
            .map(|list| list.id))
    }

    /// A new, enabled, unlocked list (every actor may create one).
    pub fn create_list(&self, _actor: Actor, name: &str) -> Result<ListId, MaskError> {
        self.change(|doc| admin::create_list(doc, name))
    }

    /// Adds entries; an equal existing entry is returned instead of a copy.
    pub fn add_entries(
        &self,
        actor: Actor,
        list: ListId,
        entries: Vec<NewEntry>,
    ) -> Result<Vec<EntrySummary>, MaskError> {
        self.change(|doc| admin::add_entries(doc, actor, list, entries))
    }

    /// Adds a protected path; returns its stored form.
    pub fn add_scope(&self, actor: Actor, list: ListId, scope: &str) -> Result<String, MaskError> {
        self.change(|doc| admin::add_scope(doc, actor, list, scope))
    }

    pub fn set_locked(&self, actor: Actor, list: ListId, locked: bool) -> Result<(), MaskError> {
        self.change(|doc| admin::set_locked(doc, actor, list, locked))
    }

    pub fn set_enabled(&self, actor: Actor, list: ListId, enabled: bool) -> Result<(), MaskError> {
        self.change(|doc| admin::set_enabled(doc, actor, list, enabled))
    }

    pub fn remove_entry(
        &self,
        actor: Actor,
        list: ListId,
        entry: EntryId,
    ) -> Result<(), MaskError> {
        self.change(|doc| admin::remove_entry(doc, actor, list, entry))
    }

    pub fn remove_scope(&self, actor: Actor, list: ListId, scope: &str) -> Result<(), MaskError> {
        self.change(|doc| admin::remove_scope(doc, actor, list, scope))
    }

    pub fn delete_list(&self, actor: Actor, list: ListId) -> Result<(), MaskError> {
        self.change(|doc| admin::delete_list(doc, actor, list))
    }

    pub fn update_entry(
        &self,
        actor: Actor,
        list: ListId,
        entry: EntryId,
        update: EntryUpdate,
    ) -> Result<EntrySummary, MaskError> {
        self.change(|doc| admin::update_entry(doc, actor, list, entry, update))
    }

    /// Entries with clear text and their assigned values: only `Gui`.
    pub fn reveal(&self, actor: Actor, list: ListId) -> Result<Vec<EntryWithValue>, MaskError> {
        let doc = self.load()?.unwrap_or_else(StoreDoc::new);
        admin::reveal(&doc, actor, list)
    }

    /// Removes the whole store (files, key, side records): the way out of an
    /// unreadable store. Only `VerifiedUser`.
    pub fn reset(&self, actor: Actor) -> Result<(), MaskError> {
        if actor != Actor::VerifiedUser {
            return Err(MaskError::NotAllowed(
                "den Maskenspeicher zurücksetzen braucht die Bestätigung des Nutzers".into(),
            ));
        }
        let _guard = self.write_guard()?;
        for name in [STATE_FILE, SEALED_FILE] {
            match std::fs::remove_file(self.path(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(MaskError::Io(format!("{name} löschen: {error}"))),
            }
        }
        for account in [side::LOCKDOWN, side::GENERATION, side::MARKER, side::KEY] {
            self.inner.side.delete(account).map_err(MaskError::Io)?;
        }
        self.invalidate();
        Ok(())
    }

    /// One read-change-write under the write guard.
    fn change<T>(
        &self,
        apply: impl FnOnce(&mut StoreDoc) -> Result<T, MaskError>,
    ) -> Result<T, MaskError> {
        let _guard = self.write_guard()?;
        let mut doc = self.load()?.unwrap_or_else(StoreDoc::new);
        let before = doc.clone();
        let value = apply(&mut doc)?;
        if doc != before {
            self.save(&mut doc, &before)?;
            self.invalidate();
        }
        Ok(value)
    }
}
