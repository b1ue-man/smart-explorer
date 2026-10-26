//! PackageManager icons registered locally by the Android app, never by a remote peer.
use crate::screen::core::app_icon::{AuthorizedIconEntry, IconRef, IconReply};
use std::collections::BTreeMap;
use std::io;
use std::sync::{Mutex, OnceLock, PoisonError};

const MAX_REGISTERED_BYTES: usize = 8 * 1024 * 1024;
const MAX_REGISTERED_ICONS: usize = 128;

fn store() -> &'static Mutex<BTreeMap<String, IconReply>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, IconReply>>> = OnceLock::new();
    STORE.get_or_init(Mutex::default)
}

/// Only the local hostPrereqs bridge calls this with its current PackageManager snapshot.
/// Replacing the snapshot also removes packages that ceased to be visible to the app.
pub(crate) fn replace_registered(icons: Vec<IconReply>) -> io::Result<()> {
    let mut next = BTreeMap::new();
    let mut retained = 0usize;
    for reply in icons.into_iter().take(MAX_REGISTERED_ICONS) {
        // An unavailable/broken app icon never rejects host registration or projection.
        let Ok(bytes) = reply.bytes_for(&reply.icon) else {
            continue;
        };
        if crate::screen::core::app_icon::decode_png(&bytes).is_err() {
            continue;
        }
        retained = retained.saturating_add(bytes.len());
        if retained > MAX_REGISTERED_BYTES {
            break;
        }
        next.insert(reply.icon.app_id.clone(), reply);
    }
    *store().lock().unwrap_or_else(PoisonError::into_inner) = next;
    Ok(())
}

pub(crate) fn reference(app_id: &str) -> Option<IconRef> {
    store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(app_id)
        .map(|reply| reply.icon.clone())
}

pub(crate) fn load(entry: AuthorizedIconEntry<'_>) -> io::Result<Option<IconReply>> {
    let (id, expected) = match entry {
        AuthorizedIconEntry::App(app) => (Some(app.app_id.as_str()), app.icon.as_ref()),
        AuthorizedIconEntry::Window(window) => (window.app_id.as_deref(), window.icon.as_ref()),
    };
    let (Some(id), Some(expected)) = (id, expected) else {
        return Ok(None);
    };
    Ok(store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(id)
        .filter(|reply| &reply.icon == expected)
        .cloned())
}
