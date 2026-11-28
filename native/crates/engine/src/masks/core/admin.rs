//! Management of the lists on the store document, with the rights of each
//! actor (`rights`). Every function either changes the document and raises
//! its `config_generation` or leaves it untouched and returns the error.
use super::doc::{EntryDoc, ListDoc, StoreDoc};
use super::fuzzy::FuzzyPattern;
use super::model::{
    validate_label, validate_name, Actor, EntryId, EntryKind, EntrySummary, EntryUpdate,
    EntryWithValue, ListId, ListSummary, MaskError, NewEntry, Tolerance, DEFAULT_LABEL,
};
use super::placeholder;
use super::rights::{check, check_reveal, classify_update, Change};
use super::scope::Scope;

/// Byte limit of the compiled program of one regex entry.
pub(crate) const REGEX_SIZE_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) fn create_list(doc: &mut StoreDoc, name: &str) -> Result<ListId, MaskError> {
    let name = validate_name(name)?;
    let folded = name.to_lowercase();
    if doc
        .lists
        .iter()
        .any(|list| list.name.to_lowercase() == folded)
    {
        return Err(MaskError::DuplicateName(name));
    }
    let id = ListId(doc.next_list.max(1));
    doc.next_list = id.0 + 1;
    doc.lists.push(ListDoc {
        id,
        name,
        enabled: true,
        locked: false,
        scopes: Vec::new(),
        entries: Vec::new(),
        assigned: Vec::new(),
    });
    doc.config_generation += 1;
    Ok(id)
}

pub(crate) fn add_entries(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    entries: Vec<NewEntry>,
) -> Result<Vec<EntrySummary>, MaskError> {
    let current = doc.list(list).ok_or(MaskError::NoSuchList(list))?;
    check(actor, current, Change::Strengthen)?;
    let mut prepared = Vec::with_capacity(entries.len());
    for entry in entries {
        prepared.push(prepare(entry)?);
    }
    let mut next = doc.next_entry.max(1);
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    let mut summaries = Vec::with_capacity(prepared.len());
    let mut changed = false;
    for mut entry in prepared {
        if let Some(existing) = list_doc.entries.iter().find(|e| same_entry(e, &entry)) {
            summaries.push(summary(existing));
            continue;
        }
        entry.id = EntryId(next);
        next += 1;
        summaries.push(summary(&entry));
        list_doc.entries.push(entry);
        changed = true;
    }
    if changed {
        doc.next_entry = next;
        doc.config_generation += 1;
    }
    Ok(summaries)
}

pub(crate) fn add_scope(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    scope: &str,
) -> Result<String, MaskError> {
    let scope = Scope::parse(scope)?;
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    check(actor, list_doc, Change::Strengthen)?;
    let text = scope.as_str().to_string();
    if !list_doc.scopes.contains(&scope) {
        list_doc.scopes.push(scope);
        doc.config_generation += 1;
    }
    Ok(text)
}

pub(crate) fn set_locked(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    locked: bool,
) -> Result<(), MaskError> {
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    if list_doc.locked == locked {
        return Ok(());
    }
    let change = if locked {
        Change::Strengthen
    } else {
        Change::Weaken
    };
    // Locking a list is always allowed, also to the agent.
    if change == Change::Weaken {
        check(actor, list_doc, change)?;
    }
    list_doc.locked = locked;
    doc.config_generation += 1;
    Ok(())
}

pub(crate) fn set_enabled(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    enabled: bool,
) -> Result<(), MaskError> {
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    if list_doc.enabled == enabled {
        return Ok(());
    }
    let change = if enabled {
        Change::Strengthen
    } else {
        Change::Weaken
    };
    check(actor, list_doc, change)?;
    list_doc.enabled = enabled;
    doc.config_generation += 1;
    Ok(())
}

pub(crate) fn remove_entry(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    entry: EntryId,
) -> Result<(), MaskError> {
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    check(actor, list_doc, Change::Weaken)?;
    let before = list_doc.entries.len();
    list_doc.entries.retain(|e| e.id != entry);
    if list_doc.entries.len() == before {
        return Err(MaskError::NoSuchEntry(entry));
    }
    // Assigned placeholders stay, so earlier outputs still resolve.
    doc.config_generation += 1;
    Ok(())
}

pub(crate) fn remove_scope(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    scope: &str,
) -> Result<(), MaskError> {
    let wanted = Scope::parse(scope)?;
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    check(actor, list_doc, Change::Weaken)?;
    let before = list_doc.scopes.len();
    list_doc.scopes.retain(|s| *s != wanted);
    if list_doc.scopes.len() == before {
        return Err(MaskError::NoSuchScope(wanted.as_str().to_string()));
    }
    doc.config_generation += 1;
    Ok(())
}

pub(crate) fn delete_list(doc: &mut StoreDoc, actor: Actor, list: ListId) -> Result<(), MaskError> {
    let list_doc = doc.list(list).ok_or(MaskError::NoSuchList(list))?;
    check(actor, list_doc, Change::Weaken)?;
    doc.lists.retain(|l| l.id != list);
    doc.config_generation += 1;
    Ok(())
}

pub(crate) fn update_entry(
    doc: &mut StoreDoc,
    actor: Actor,
    list: ListId,
    entry: EntryId,
    update: EntryUpdate,
) -> Result<EntrySummary, MaskError> {
    let list_doc = doc.list(list).ok_or(MaskError::NoSuchList(list))?;
    let current = list_doc
        .entries
        .iter()
        .find(|e| e.id == entry)
        .ok_or(MaskError::NoSuchEntry(entry))?;
    check(actor, list_doc, classify_update(current, &update))?;
    let mut changed = NewEntry {
        kind: current.kind,
        value: update.value.unwrap_or_else(|| current.value.clone()),
        label: Some(update.label.unwrap_or_else(|| current.label.clone())),
        case_sensitive: update.case_sensitive.unwrap_or(current.case_sensitive),
        whole_word: update.whole_word.unwrap_or(current.whole_word),
        tolerance: update.tolerance.unwrap_or(current.tolerance),
    };
    if changed.kind != EntryKind::Fuzzy {
        changed.tolerance = Tolerance::Off;
    }
    let mut prepared = prepare(changed)?;
    prepared.id = entry;
    let list_doc = doc.list_mut(list).ok_or(MaskError::NoSuchList(list))?;
    let slot = list_doc
        .entries
        .iter_mut()
        .find(|e| e.id == entry)
        .ok_or(MaskError::NoSuchEntry(entry))?;
    let result = summary(&prepared);
    if *slot != prepared {
        *slot = prepared;
        doc.config_generation += 1;
    }
    Ok(result)
}

pub(crate) fn summaries(doc: &StoreDoc) -> Vec<ListSummary> {
    doc.lists
        .iter()
        .map(|list| ListSummary {
            id: list.id,
            name: list.name.clone(),
            enabled: list.enabled,
            locked: list.locked,
            scopes: list.scopes.iter().map(|s| s.as_str().to_string()).collect(),
            entries: list.entries.iter().map(summary).collect(),
            assigned_values: list.assigned.len(),
        })
        .collect()
}

pub(crate) fn reveal(
    doc: &StoreDoc,
    actor: Actor,
    list: ListId,
) -> Result<Vec<EntryWithValue>, MaskError> {
    check_reveal(actor)?;
    let list_doc = doc.list(list).ok_or(MaskError::NoSuchList(list))?;
    Ok(list_doc
        .entries
        .iter()
        .map(|entry| {
            let mut assigned = Vec::new();
            for value in list_doc.assigned.iter().filter(|a| a.entry == entry.id) {
                let mut text = Vec::new();
                placeholder::write(&value.label, value.number, None, b"", &mut text);
                assigned.push((
                    String::from_utf8_lossy(&value.value.0).into_owned(),
                    String::from_utf8_lossy(&text).into_owned(),
                ));
                for (index, variant) in value.variants.iter().enumerate() {
                    let mut text = Vec::new();
                    let number = Some(index as u64 + 1);
                    placeholder::write(&value.label, value.number, number, b"", &mut text);
                    assigned.push((
                        String::from_utf8_lossy(&variant.0).into_owned(),
                        String::from_utf8_lossy(&text).into_owned(),
                    ));
                }
            }
            EntryWithValue {
                entry: summary(entry),
                value: entry.value.clone(),
                assigned,
            }
        })
        .collect())
}

pub(crate) fn summary(entry: &EntryDoc) -> EntrySummary {
    EntrySummary {
        id: entry.id,
        kind: entry.kind,
        label: entry.label.clone(),
        case_sensitive: entry.case_sensitive,
        whole_word: entry.whole_word,
        tolerance: entry.tolerance,
    }
}

fn same_entry(a: &EntryDoc, b: &EntryDoc) -> bool {
    a.kind == b.kind
        && a.value == b.value
        && a.label == b.label
        && a.case_sensitive == b.case_sensitive
        && a.whole_word == b.whole_word
        && a.tolerance == b.tolerance
}

/// Validates a new entry and brings it into its stored form (id 0 until
/// it is placed).
pub(crate) fn prepare(entry: NewEntry) -> Result<EntryDoc, MaskError> {
    let label = entry
        .label
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| DEFAULT_LABEL.to_string());
    validate_label(&label)?;
    if entry.value.is_empty() {
        return Err(MaskError::InvalidEntry("leerer Wert".into()));
    }
    match entry.kind {
        EntryKind::Literal => {}
        EntryKind::Regex => {
            regex::bytes::RegexBuilder::new(&entry.value)
                .size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map_err(|error| MaskError::InvalidEntry(format!("Regex: {error}")))?;
        }
        EntryKind::Fuzzy => {
            if FuzzyPattern::new(&entry.value, entry.tolerance).is_none() {
                return Err(MaskError::InvalidEntry(
                    "ein unscharfer Eintrag braucht Buchstaben oder Ziffern".into(),
                ));
            }
        }
    }
    let literal = entry.kind == EntryKind::Literal;
    Ok(EntryDoc {
        id: EntryId(0),
        kind: entry.kind,
        value: entry.value,
        label,
        case_sensitive: literal && entry.case_sensitive,
        whole_word: literal && entry.whole_word,
        tolerance: if entry.kind == EntryKind::Fuzzy {
            entry.tolerance
        } else {
            Tolerance::Off
        },
    })
}
