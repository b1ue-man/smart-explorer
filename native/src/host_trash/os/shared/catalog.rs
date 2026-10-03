//! Bounded paging of durable intents; each broken entry keeps its own error.
use std::{cmp::Reverse, collections::BinaryHeap, io};
use super::{platform::{self, Location}, record::{self, CatalogEntry, CatalogPage, EntryState}, store::Store};

pub(crate) fn list(cursor: Option<&str>) -> io::Result<CatalogPage> {
    let store = Store::open()?;
    list_in(&store, cursor)
}
pub(super) fn list_in(store: &Store, cursor: Option<&str>) -> io::Result<CatalogPage> {
    if cursor.is_some_and(|cursor| !record::lower_hex(cursor, 32)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Ungültiger Papierkorb-Seitenanker"));
    }
    let mut page = CatalogPage::default();
    // Keep only this page plus its successor marker, never all filenames.
    let mut selected = BinaryHeap::new();
    for entry in store.area.directory.read_directory()? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => { issue(&mut page, format!("Eintrag nicht lesbar: {error}")); continue; }
        };
        let Some(name) = entry.name.to_str() else { issue(&mut page, "Eintragname nicht darstellbar".into()); continue; };
        if name == "operation.lock" || crate::vfs::is_staging_name(name) { continue; }
        let Some(id) = name.strip_suffix(".json").filter(|id| record::lower_hex(id, 32)) else {
            issue(&mut page, format!("Unbekannter Store-Eintrag: {name}")); continue;
        };
        if cursor.is_some_and(|cursor| id >= cursor) { continue; }
        selected.push(Reverse(id.to_owned()));
        if selected.len() > record::PAGE_SIZE + 1 { selected.pop(); }
    }
    let mut ids: Vec<String> = selected.into_sorted_vec().into_iter().map(|id| id.0).collect();
    if ids.len() > record::PAGE_SIZE { ids.pop(); page.next = ids.last().cloned(); }
    for id in ids {
        let entry = match store.load(&id) {
            Ok(record) => {
                let original = crate::local_access::display_path(&platform::original_path(&record));
                let (state, detail) = match platform::locate(&record) {
                    Ok(Location::Held(held)) => (if held.slot == 0 { EntryState::Held } else { EntryState::RestorePending }, None),
                    Ok(Location::Original) => (EntryState::OriginalPresent, None),
                    Ok(Location::Missing) => (EntryState::Missing, Some("Keine bestätigte Datei an Original- oder Held-Position".into())),
                    Err(error) => (EntryState::Problem, Some(error.to_string())),
                };
                CatalogEntry { id, original, size:Some(record.size), created_ms:Some(record.created_ms), state, detail }
            }
            Err(error) => CatalogEntry { original:format!("Wiederherstellungsrecord {id}"),
                id, size:None, created_ms:None, state:EntryState::Problem,
                detail:Some(format!("Intent unvollständig oder nicht lesbar: {error}; es wird keine Datei verändert")) },
        };
        page.entries.push(entry);
    }
    Ok(page)
}
fn issue(page: &mut CatalogPage, text: String) {
    if page.issues.len() < record::PAGE_SIZE { page.issues.push(text); }
    else { page.suppressed_issues = page.suppressed_issues.saturating_add(1); }
}
