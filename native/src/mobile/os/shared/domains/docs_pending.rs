//! Counts only RAM for held engines; unopened caches use the read-only audit.
//! Unknown inventory is null, with a separately named known lower bound.
use super::{cache_path, engine, lock, opened, table, EngineKey, Slot, ROOT_FILE_SUFFIX};
use crate::mount::MountId;
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};

pub(super) fn pending() -> Value {
    let (keys, mut unknown) = keys();
    let mut known = 0usize;
    for (key, id) in keys {
        match waiting(&key, &id) {
            Ok(Some(count)) => known = known.saturating_add(count),
            Ok(None) | Err(_) => {
                unknown.insert(key.label());
            }
        }
    }
    json!({ "pending": if unknown.is_empty() { Some(known) } else { None },
        "knownPending": known, "unknown": unknown })
}

pub(super) fn retry_pending(released: &dyn Fn(&super::RootId) -> bool) -> Vec<String> {
    let (keys, _) = keys();
    let mut failed = Vec::new();
    for (key, id) in keys {
        let waiting = waiting(&key, &id).ok().flatten();
        let open = opened(&key);
        if !released(&key.root()) {
            if waiting != Some(0) {
                failed.push(key.label());
            }
            continue;
        }
        if open.is_none() && waiting == Some(0) {
            continue;
        }
        // Unknown does not mean empty: only the authorized opener can decrypt
        // and repair it. Drop the observation gate before engine takes it.
        let result = match open {
            Some(slot) => slot.fs.retry_pending_changes(),
            None => engine(&key).and_then(|slot| slot.fs.retry_pending_changes()),
        };
        if result.is_err() {
            failed.push(key.label());
        }
    }
    failed.sort();
    failed.dedup();
    failed
}

fn held(key: &EngineKey) -> Option<Arc<Slot>> {
    let table = lock(table());
    table
        .open
        .get(key)
        .cloned()
        .or_else(|| table.retired.get(key).and_then(std::sync::Weak::upgrade))
}

fn waiting(key: &EngineKey, id: &MountId) -> io::Result<Option<usize>> {
    let gate = Arc::clone(lock(table()).opening.entry(key.clone()).or_default());
    let _serial = gate.try_lock().map_err(|_| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "Dokumentcache wird gerade geöffnet; Anzahl bleibt unbekannt",
        )
    })?;
    if let Some(slot) = held(key) {
        return slot.pending_count().map(Some);
    }
    crate::mount::os::linux_os::audit_pending_state(&cache_path()?, id)
}

fn keys() -> (HashMap<EngineKey, MountId>, BTreeSet<String>) {
    let mut keys = HashMap::new();
    let mut unknown = BTreeSet::new();
    let held: Vec<_> = {
        let table = lock(table());
        table
            .open
            .keys()
            .cloned()
            .chain(
                table
                    .retired
                    .iter()
                    .filter(|(_, slot)| slot.strong_count() > 0)
                    .map(|(key, _)| key.clone()),
            )
            .collect()
    };
    for key in held {
        match key.mount_id() {
            Ok(id) => {
                keys.insert(key, id);
            }
            Err(_) => {
                unknown.insert(key.label());
            }
        }
    }
    let (records, incomplete) = recorded();
    keys.extend(records);
    if incomplete {
        unknown.insert(
            "Cacheinventur nicht vollständig lesbar; lokale Änderungen bleiben erhalten".into(),
        );
    }
    (keys, unknown)
}

/// A broken cache/marker cannot silently turn the entire inventory into zero.
fn recorded() -> (HashMap<EngineKey, MountId>, bool) {
    let Ok(cache) = cache_path() else {
        return (HashMap::new(), true);
    };
    let entries = match std::fs::read_dir(&cache) {
        Ok(entries) => entries,
        Err(error) => return (HashMap::new(), error.kind() != io::ErrorKind::NotFound),
    };
    let mut roots = HashMap::new();
    let mut directories = Vec::new();
    let mut incomplete = false;
    for entry in entries {
        let Ok(entry) = entry else {
            incomplete = true;
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else {
            incomplete = true;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            incomplete = true;
            continue;
        };
        if kind.is_symlink() {
            incomplete = true;
            continue;
        }
        if kind.is_dir() && name.starts_with("docs-") {
            match MountId::parse(&name) {
                Ok(id) => directories.push(id),
                Err(_) => incomplete = true,
            }
        }
        let Some(id) = name.strip_suffix(ROOT_FILE_SUFFIX) else {
            continue;
        };
        let parsed = (|| {
            if !kind.is_file() {
                return Err(invalid());
            }
            let id = MountId::parse(id)?;
            let label = crate::support_dirs::read_private_text(&entry.path(), 4096)?;
            let key = EngineKey::from_label(label.trim()).ok_or_else(invalid)?;
            if key.mount_id()? != id {
                return Err(invalid());
            }
            Ok((key, id))
        })();
        match parsed {
            Ok((key, id)) => {
                roots.insert(key, id);
            }
            Err(_) => incomplete = true,
        }
    }
    if directories
        .iter()
        .any(|id| !roots.values().any(|known| known == id))
    {
        incomplete = true;
    }
    (roots, incomplete)
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Dokumentcache besitzt keine vollständige stabile Rootzuordnung",
    )
}
