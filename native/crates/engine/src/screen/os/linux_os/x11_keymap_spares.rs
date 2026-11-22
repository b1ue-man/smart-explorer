//! Spare keys whose emptying failed (review 2026-11-19, Codex 10/16): a
//! text gave a free keycode a keysym for one press and could not empty it
//! again (a lost connection). They are kept in memory and in
//! `<app data>/x11-spare-keys` (one line per key: display, keycode, keysym),
//! so the next host process on the same display empties them too, and are
//! emptied with the next X connection or text, each only while the key still
//! carries exactly that keysym (another X server, or a client that took the
//! key since, keeps its mapping). The host process keeps its X connection in
//! a static for its whole life, so no holder is dropped at its end; the file
//! carries a debt across that end.
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym};

use super::{sync, KeyMap, NO_SYMBOL};

/// The keys owed, with the display they belong to; `None` until the file
/// was read.
fn owed() -> MutexGuard<'static, Option<Vec<(String, Keycode, Keysym)>>> {
    static OWED: Mutex<Option<Vec<(String, Keycode, Keysym)>>> = Mutex::new(None);
    OWED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn file() -> std::path::PathBuf {
    crate::support_dirs::app_data_file("x11-spare-keys")
}

/// The display this process's connections go to (`RustConnection::connect(None)`).
fn display() -> String {
    std::env::var("DISPLAY").unwrap_or_default()
}

/// The owed keys, read from the file once per process.
fn loaded(
    owed: &mut Option<Vec<(String, Keycode, Keysym)>>,
) -> &mut Vec<(String, Keycode, Keysym)> {
    owed.get_or_insert_with(|| {
        std::fs::read_to_string(file())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let display = fields.next()?.to_string();
                let code = fields.next()?.parse().ok()?;
                let sym = fields.next()?.parse().ok()?;
                Some((display, code, sym))
            })
            .collect()
    })
}

/// Writes the owed keys back (no file while nothing is owed).
fn store(owed: &[(String, Keycode, Keysym)]) {
    let path = file();
    if owed.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    let text: String = owed
        .iter()
        .map(|(display, code, sym)| format!("{display}\t{code}\t{sym}\n"))
        .collect();
    if let Err(error) = std::fs::write(&path, text) {
        eprintln!("screen host process: owed spare keys not stored: {error}");
    }
}

/// `spare` still carries `sym`: owed until a connection empties it.
pub(super) fn owe_spare(spare: Keycode, sym: Keysym) {
    let mut owed = owed();
    let list = loaded(&mut owed);
    list.push((display(), spare, sym));
    store(list);
}

/// Gives `spare` its empty mapping back.
pub(super) fn empty_spare<C: Connection>(conn: &C, spare: Keycode, per: u8) -> io::Result<()> {
    let empty = vec![NO_SYMBOL; usize::from(per)];
    conn.change_keyboard_mapping(1, spare, per, &empty)
        .map_err(io::Error::other)?;
    sync(conn)
}

/// Empties the keys owed on this display that still carry their keysym
/// (with every new X connection and every text); the rest stays owed.
pub(in crate::screen::os) fn restore_pending<C: Connection>(conn: &C) {
    let mut owed = owed();
    let list = loaded(&mut owed);
    let here = display();
    if !list.iter().any(|(display, _, _)| *display == here) {
        return;
    }
    let Ok(map) = KeyMap::read(conn) else {
        return;
    };
    let before = list.len();
    list.retain(|(display, spare, sym)| {
        if *display != here {
            return true;
        }
        let index = usize::from(spare.saturating_sub(map.min));
        let ours = index < map.codes() && map.row(index).iter().all(|carried| carried == sym);
        // Not ours any more: nothing to give back. Ours and emptied: done.
        ours && empty_spare(conn, *spare, map.per_code.max(1)).is_err()
    });
    if list.len() != before {
        store(list);
    }
}
