//! Spare keys a text remapped (review 2026-11-19, Codex 10/16, round 11):
//! a text gives a free keycode a keysym for one press and empties it again.
//! A key is owed in `<app data>/x11-spare-keys` before its first remapping
//! (`claim`), so a host process killed between remapping and emptying
//! leaves its debt behind, and cleared after the text (`settle`) unless its
//! emptying failed (`owe`). Every new X connection and every text empty the
//! keys owed on their display (`restore_pending`), each only while it still
//! looks like se's (a keysym in the first column, the others the same or
//! none; with a known keysym its short salted hash must match too), so
//! another X server's or another client's mapping stays. The file names no
//! typed character: a keycode, a salt and an 8-bit hash (round 11, N6;
//! about 4000 code points share each hash). Every change reads, merges and
//! replaces the file under an exclusive lock (`File::lock`) by renaming a
//! complete copy over it (N3), as two host processes may share it.
use std::hash::{Hash, Hasher};
use std::io::{self, Write};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym};

use super::{sync, KeyMap, NO_SYMBOL};

/// One owed key: its display, keycode, salt and the hash of the keysym it
/// carries (`None`: claimed before a character, any keysym).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owed {
    display: String,
    code: Keycode,
    salt: u32,
    hash: Option<u8>,
}

fn file(name: &str) -> std::path::PathBuf {
    crate::support_dirs::app_data_file(name)
}

/// The display this process's connections go to (`RustConnection::connect(None)`).
fn display() -> String {
    std::env::var("DISPLAY").unwrap_or_default()
}

fn hash(salt: u32, sym: Keysym) -> u8 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (salt, sym).hash(&mut hasher);
    (hasher.finish() & 0xFF) as u8
}

fn salt() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |at| at.subsec_nanos());
    nanos ^ std::process::id()
}

fn parse(text: &str) -> Vec<Owed> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            Some(Owed {
                display: fields.next()?.to_string(),
                code: fields.next()?.parse().ok()?,
                salt: fields.next()?.parse().ok()?,
                hash: match fields.next()? {
                    "-" => None,
                    value => Some(value.parse().ok()?),
                },
            })
        })
        .collect()
}

/// Reads, changes and writes the owed keys under the file's lock.
fn change(apply: impl FnOnce(&mut Vec<Owed>)) {
    let changed = (|| -> io::Result<()> {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(file("x11-spare-keys.lock"))?;
        lock.lock()?;
        let path = file("x11-spare-keys");
        let mut owed = parse(&std::fs::read_to_string(&path).unwrap_or_default());
        let before = owed.clone();
        apply(&mut owed);
        if owed == before {
            return Ok(());
        }
        if owed.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let staged = file("x11-spare-keys.new");
        let mut out = std::fs::File::create(&staged)?;
        for entry in &owed {
            let hash = entry
                .hash
                .map_or_else(|| "-".into(), |hash| hash.to_string());
            writeln!(
                out,
                "{}\t{}\t{}\t{hash}",
                entry.display, entry.code, entry.salt
            )?;
        }
        out.sync_all()?;
        std::fs::rename(staged, path)
    })();
    if let Err(error) = changed {
        eprintln!("screen host process: owed spare keys not stored: {error}");
    }
}

/// `spare` is about to be remapped: owed until `settle`.
pub(super) fn claim(spare: Keycode) {
    let display = display();
    change(|owed| {
        if !owed
            .iter()
            .any(|entry| entry.display == display && entry.code == spare)
        {
            owed.push(Owed {
                display,
                code: spare,
                salt: salt(),
                hash: None,
            });
        }
    });
}

/// `spare` still carries `sym`: owed with its hash.
pub(super) fn owe(spare: Keycode, sym: Keysym) {
    let display = display();
    change(|owed| {
        owed.retain(|entry| !(entry.display == display && entry.code == spare));
        let salt = salt();
        owed.push(Owed {
            display,
            code: spare,
            salt,
            hash: Some(hash(salt, sym)),
        });
    });
}

/// The text ended: its claims are cleared, except the keys `owed`.
pub(super) fn settle(claimed: &[Keycode], owed_keys: &[Keycode]) {
    if claimed.iter().all(|code| owed_keys.contains(code)) {
        return;
    }
    let display = display();
    change(|owed| {
        owed.retain(|entry| {
            entry.display != display
                || !claimed.contains(&entry.code)
                || owed_keys.contains(&entry.code)
        });
    });
}

/// Gives `spare` its empty mapping back.
pub(super) fn empty_spare<C: Connection>(conn: &C, spare: Keycode, per: u8) -> io::Result<()> {
    let empty = vec![NO_SYMBOL; usize::from(per)];
    conn.change_keyboard_mapping(1, spare, per, &empty)
        .map_err(io::Error::other)?;
    sync(conn)
}

/// The key still looks like se's (N5): a keysym in the first column, the
/// others the same or none, and with a known hash that keysym's hash.
fn ours(map: &KeyMap, entry: &Owed) -> bool {
    let index = usize::from(entry.code.saturating_sub(map.min));
    if index >= map.codes() {
        return false;
    }
    let row = map.row(index);
    let Some(&first) = row.first().filter(|first| **first != NO_SYMBOL) else {
        return false;
    };
    row.iter().all(|sym| *sym == first || *sym == NO_SYMBOL)
        && entry
            .hash
            .is_none_or(|hash| self::hash(entry.salt, first) == hash)
}

/// Empties the keys owed on this display that still look like se's; those
/// whose emptying fails stay owed.
pub(in crate::screen::os) fn restore_pending<C: Connection>(conn: &C) {
    let here = display();
    let path = file("x11-spare-keys");
    let owed = parse(&std::fs::read_to_string(path).unwrap_or_default());
    if !owed.iter().any(|entry| entry.display == here) {
        return;
    }
    let Ok(map) = KeyMap::read(conn) else {
        return;
    };
    change(|owed| {
        owed.retain(|entry| {
            if entry.display != here {
                return true;
            }
            ours(&map, entry) && empty_spare(conn, entry.code, map.per_code.max(1)).is_err()
        });
    });
}
