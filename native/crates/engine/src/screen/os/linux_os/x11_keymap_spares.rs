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
//! complete copy over it (N3), as two host processes may share it. Each
//! entry names its owner (process id and start time; round 12, S2 and
//! Codex 11): a claim (no hash yet) stays while its owner lives, as it may
//! be typing on the key right now; a key a process owes after a failed
//! emptying (with its hash) may be emptied by any process. The emptying is
//! checked (Codex 10): an entry leaves the file only once the server took
//! it.
use std::io::{self, Write};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym};

use super::{KeyMap, NO_SYMBOL};

/// One owed key: its display, keycode, salt and the hash of the keysym it
/// carries (`None`: claimed before a character, any keysym), and its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owed {
    display: String,
    code: Keycode,
    salt: u32,
    hash: Option<u8>,
    owner: Option<Owner>,
}

/// A process: its id and its start time in clock ticks since boot
/// (`/proc/<pid>/stat` field 22), so a reused id names another process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    pid: u32,
    start: u64,
}

impl Owner {
    fn of(pid: u32) -> Option<Self> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name in field 2 may hold spaces; field 22 is the
        // 20th after its closing parenthesis.
        let rest = &stat[stat.rfind(')')? + 1..];
        let start = rest.split_whitespace().nth(19)?.parse().ok()?;
        Some(Self { pid, start })
    }
    /// This process (read once: its id and start time do not change).
    fn me() -> Option<Self> {
        static ME: std::sync::OnceLock<Option<Owner>> = std::sync::OnceLock::new();
        *ME.get_or_init(|| Self::of(std::process::id()))
    }
    fn alive(self) -> bool {
        Self::of(self.pid) == Some(self)
    }
}

fn file(name: &str) -> std::path::PathBuf {
    crate::support_dirs::app_data_file(name)
}

/// The display this process's connections go to (`RustConnection::connect(None)`).
fn display() -> String {
    std::env::var("DISPLAY").unwrap_or_default()
}

/// FNV-1a (32 bit) of salt and keysym, its low byte (round 12, S6): the
/// same in every build, unlike `DefaultHasher` ("the internal algorithm is
/// not specified, and so it and its hashes should not be relied upon over
/// releases"). A hash an older build stored no longer matches: that entry
/// leaves the file without emptying its key.
fn hash(salt: u32, sym: Keysym) -> u8 {
    let mut value: u32 = 0x811c_9dc5;
    for byte in salt.to_le_bytes().into_iter().chain(sym.to_le_bytes()) {
        value ^= u32::from(byte);
        value = value.wrapping_mul(0x0100_0193);
    }
    (value & 0xFF) as u8
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
                // Without an owner (an older line) the entry counts as left.
                owner: match (fields.next(), fields.next()) {
                    (Some(pid), Some(start)) => Some(Owner {
                        pid: pid.parse().ok()?,
                        start: start.parse().ok()?,
                    }),
                    _ => None,
                },
            })
        })
        .collect()
}

/// One entry as its line of the file (`parse` reads it back).
fn line(entry: &Owed) -> String {
    let hash = entry
        .hash
        .map_or_else(|| "-".into(), |hash| hash.to_string());
    let owner = entry.owner.map_or_else(String::new, |owner| {
        format!("\t{}\t{}", owner.pid, owner.start)
    });
    format!(
        "{}\t{}\t{}\t{hash}{owner}",
        entry.display, entry.code, entry.salt
    )
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
            writeln!(out, "{}", line(entry))?;
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
                owner: Owner::me(),
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
            owner: Owner::me(),
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

/// Gives `spare` its empty mapping back; an error when the server refused
/// it (Codex 10: checked, not left to the unread event queue).
pub(super) fn empty_spare<C: Connection>(conn: &C, spare: Keycode, per: u8) -> io::Result<()> {
    let empty = vec![NO_SYMBOL; usize::from(per)];
    conn.change_keyboard_mapping(1, spare, per, &empty)
        .map_err(io::Error::other)?
        .check()
        .map_err(io::Error::other)
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

/// Whether this process may empty `entry` now. A claim (`hash` still
/// `None`) is in use while its owner lives: this process's own one belongs
/// to its running text (a text settles or owes its claims before it
/// returns, and one process types one text at a time under its input
/// lock), another living process's one may be typing on the key (S2,
/// Codex 11). A key owed after a failed emptying (`hash` known) is no
/// longer typed on: any process may empty it while its hash still
/// matches. An entry without an owner (an older build's) counts as left.
fn restorable(entry: &Owed, me: Option<Owner>, alive: impl Fn(Owner) -> bool) -> bool {
    match entry.owner {
        _ if entry.hash.is_some() => true,
        None => true,
        Some(owner) if Some(owner) == me => false,
        Some(owner) => !alive(owner),
    }
}

/// Empties the keys owed on this display that still look like se's and no
/// living process holds; those whose emptying fails stay owed.
pub(in crate::screen::os) fn restore_pending<C: Connection>(conn: &C) {
    let here = display();
    let me = Owner::me();
    let path = file("x11-spare-keys");
    let owed = parse(&std::fs::read_to_string(path).unwrap_or_default());
    if !owed
        .iter()
        .any(|entry| entry.display == here && restorable(entry, me, Owner::alive))
    {
        return;
    }
    let Ok(map) = KeyMap::read(conn) else {
        return;
    };
    change(|owed| {
        owed.retain(|entry| {
            if entry.display != here || !restorable(entry, me, Owner::alive) {
                return true;
            }
            ours(&map, entry) && empty_spare(conn, entry.code, map.per_code.max(1)).is_err()
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 12 (S2, S6, Codex 11): an entry keeps its owner through the
    /// file, an older build's line reads without one, the hash is FNV-1a
    /// (the same in every build), and only a living process's claim is
    /// left alone, this process's own included.
    #[test]
    fn screen_task_x11_spare_key_entries_owner_and_hash() {
        assert_eq!(hash(0, 0x61), 0xB4);
        assert_eq!(hash(0x1234_5678, 0x20AC), 0x19);
        let me = Owner {
            pid: 10,
            start: 500,
        };
        let other = Owner {
            pid: 11,
            start: 600,
        };
        let claim = |owner| Owed {
            display: ":0".into(),
            code: 250,
            salt: 7,
            hash: None,
            owner,
        };
        let owed = Owed {
            hash: Some(3),
            ..claim(Some(other))
        };
        for entry in [claim(Some(me)), owed.clone(), claim(None)] {
            assert_eq!(parse(&line(&entry)), vec![entry]);
        }
        assert_eq!(parse(":0\t250\t7\t-"), vec![claim(None)]);
        let living = |owner: Owner| owner == other;
        assert!(!restorable(&claim(Some(other)), Some(me), living));
        assert!(restorable(&claim(Some(other)), Some(me), |_| false));
        assert!(!restorable(&claim(Some(me)), Some(me), |_| true));
        assert!(restorable(&owed, Some(me), living));
        assert!(restorable(&claim(None), Some(me), living));
    }
}
