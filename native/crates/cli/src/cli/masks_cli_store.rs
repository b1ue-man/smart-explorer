//! The calls of `se masks` into `crate::masks::MaskStore`, with the rule of
//! the CLI: a change runs as `Actor::Cli`; when the store refuses it as
//! weakening, or the list is locked, the user confirms through the system
//! dialog (`crate::elevate::confirm_user_presence`) and the change runs as
//! `Actor::VerifiedUser`. A locked list therefore never changes through the
//! CLI without the user, even when an agent runs `se masks` in a shell.
use crate::masks::{
    Actor, EntryId, EntryKind, EntrySummary, EntryUpdate, ListId, ListSummary, MaskError,
    MaskStore, NewEntry, Tolerance,
};

pub(super) struct Store(MaskStore);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Toggle {
    Lock,
    Unlock,
    Enable,
    Disable,
}

/// Confirmation of the user through the system dialog.
pub(super) fn confirm(reason: &str) -> Result<(), String> {
    crate::elevate::confirm_user_presence(reason).map_err(|error| error.to_string())
}

pub(super) fn tolerance(text: &str) -> Result<Tolerance, String> {
    Tolerance::parse(text).ok_or_else(|| {
        format!("Toleranz „{text}“: off, low, medium, high (aus, gering, mittel, hoch) oder <Kürzung>/<Fehler innen>")
    })
}

pub(super) fn new_entry(
    kind: &str,
    value: String,
    options: &super::EntryOptions,
) -> Result<NewEntry, String> {
    let kind =
        EntryKind::parse(kind).ok_or_else(|| format!("Art „{kind}“: literal, regex oder fuzzy"))?;
    let tolerance = match &options.tolerance {
        Some(text) => tolerance(text)?,
        None if kind == EntryKind::Fuzzy => Tolerance::Medium,
        None => Tolerance::Off,
    };
    let mut entry = match kind {
        EntryKind::Literal => NewEntry::literal(value),
        EntryKind::Regex => NewEntry::regex(value),
        EntryKind::Fuzzy => NewEntry::fuzzy(value, tolerance),
    };
    entry.label = options.label.clone();
    entry.case_sensitive = options.case_sensitive.unwrap_or(false);
    entry.whole_word = options.whole_word.unwrap_or(false);
    Ok(entry)
}

fn text(error: MaskError) -> String {
    error.to_string()
}

impl Store {
    pub(super) fn open() -> Result<Self, String> {
        MaskStore::open_default().map(Self).map_err(text)
    }

    pub(super) fn lists(&self) -> Result<Vec<ListSummary>, String> {
        self.0.lists().map_err(text)
    }

    /// A list by id (digits) or by name (case-insensitive).
    pub(super) fn find(&self, list: &str) -> Result<ListSummary, String> {
        let lists = self.lists()?;
        let wanted = list.trim();
        let by_id = wanted.parse::<u64>().ok();
        lists
            .iter()
            .find(|l| Some(l.id.0) == by_id)
            .or_else(|| {
                lists
                    .iter()
                    .find(|l| l.name.to_lowercase() == wanted.to_lowercase())
            })
            .cloned()
            .ok_or_else(|| format!("keine Maskenliste „{wanted}“ (se masks list)"))
    }

    pub(super) fn summary(&self, id: ListId) -> Result<ListSummary, String> {
        self.find(&id.0.to_string())
    }

    pub(super) fn create(&self, name: &str) -> Result<ListId, String> {
        self.0.create_list(Actor::Cli, name).map_err(text)
    }

    /// Runs `change` as the CLI, or after the user's confirmation as the
    /// verified user (locked list or a weakening change).
    fn change<T>(
        &self,
        list: &ListSummary,
        reason: &str,
        change: impl Fn(Actor) -> Result<T, MaskError>,
    ) -> Result<T, String> {
        if list.locked {
            confirm(&format!("{reason} (Liste „{}“ ist gesperrt)", list.name))?;
            return change(Actor::VerifiedUser).map_err(text);
        }
        match change(Actor::Cli) {
            Err(MaskError::NotAllowed(_)) => {
                confirm(reason)?;
                change(Actor::VerifiedUser).map_err(text)
            }
            other => other.map_err(text),
        }
    }

    pub(super) fn add_entries(
        &self,
        list: &ListSummary,
        entries: Vec<NewEntry>,
    ) -> Result<Vec<EntrySummary>, String> {
        let reason = format!(
            "Smart Explorer: Einträge zur Maskenliste „{}“ hinzufügen",
            list.name
        );
        self.change(list, &reason, |actor| {
            self.0.add_entries(actor, list.id, entries.clone())
        })
    }

    pub(super) fn update_entry(
        &self,
        list: &ListSummary,
        entry: u64,
        update: EntryUpdate,
    ) -> Result<EntrySummary, String> {
        let reason = format!(
            "Smart Explorer: Eintrag {entry} der Maskenliste „{}“ ändern",
            list.name
        );
        self.change(list, &reason, |actor| {
            self.0
                .update_entry(actor, list.id, EntryId(entry), update.clone())
        })
    }

    pub(super) fn remove_entry(&self, list: &ListSummary, entry: u64) -> Result<(), String> {
        let reason = format!(
            "Smart Explorer: Eintrag {entry} aus der Maskenliste „{}“ entfernen",
            list.name
        );
        self.change(list, &reason, |actor| {
            self.0.remove_entry(actor, list.id, EntryId(entry))
        })
    }

    pub(super) fn add_scope(&self, list: &ListSummary, target: &str) -> Result<String, String> {
        let reason = format!(
            "Smart Explorer: Schutzpfad zur Maskenliste „{}“ hinzufügen",
            list.name
        );
        self.change(list, &reason, |actor| {
            self.0.add_scope(actor, list.id, target)
        })
    }

    pub(super) fn remove_scope(&self, list: &ListSummary, target: &str) -> Result<(), String> {
        let reason = format!(
            "Smart Explorer: Schutzpfad „{target}“ der Maskenliste „{}“ freigeben",
            list.name
        );
        self.change(list, &reason, |actor| {
            self.0.remove_scope(actor, list.id, target)
        })
    }

    pub(super) fn toggle(&self, list: &ListSummary, toggle: Toggle) -> Result<(), String> {
        let (what, lock, on) = match toggle {
            Toggle::Lock => ("sperren", true, true),
            Toggle::Unlock => ("entsperren", true, false),
            Toggle::Enable => ("aktivieren", false, true),
            Toggle::Disable => ("deaktivieren", false, false),
        };
        let apply = |actor: Actor| {
            if lock {
                self.0.set_locked(actor, list.id, on)
            } else {
                self.0.set_enabled(actor, list.id, on)
            }
        };
        // Locking a locked list or locking at all needs no dialog.
        if toggle == Toggle::Lock {
            return apply(Actor::Cli).map_err(text);
        }
        let reason = format!("Smart Explorer: Maskenliste „{}“ {what}", list.name);
        self.change(list, &reason, apply)
    }

    pub(super) fn delete(&self, list: &ListSummary) -> Result<(), String> {
        let reason = format!("Smart Explorer: Maskenliste „{}“ löschen", list.name);
        self.change(list, &reason, |actor| self.0.delete_list(actor, list.id))
    }
}
