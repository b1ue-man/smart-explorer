//! Text on an X11 host (SCR-M6, C5): every character is typed with the key
//! that carries its keysym in the current keyboard mapping (with Shift for
//! the second column); a character without such a key gets a spare keycode
//! (one without any keysym) for one press, after which its mapping is set
//! back to nothing. The procedure is the one xdotool uses, implemented
//! anew (NR-Nr. 69). A character no spare keycode can carry is counted and
//! skipped; its text is never logged.
use std::io;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym, Window};
use x11rb::protocol::xtest::ConnectionExt as _;

/// Event types of `XTestFakeInput`.
pub(super) const KEY_PRESS: u8 = 2;
pub(super) const KEY_RELEASE: u8 = 3;
/// `Shift_L` as a Linux code (42) plus the evdev offset 8.
const SHIFT_KEYCODE: Keycode = 50;
const NO_SYMBOL: Keysym = 0;

/// The keysym of `character` (X11 keysym encoding: Latin-1 as itself,
/// everything else as Unicode keysym).
pub(super) fn keysym_of(character: char) -> Keysym {
    match character {
        '\n' => 0xFF0D,    // Return
        '\t' => 0xFF09,    // Tab
        '\u{8}' => 0xFF08, // BackSpace
        ' '..='~' | '\u{A0}'..='\u{FF}' => character as u32,
        other => 0x0100_0000 + other as u32,
    }
}

/// The keyboard mapping as the server holds it now.
pub(super) struct KeyMap {
    pub min: Keycode,
    pub per_code: u8,
    pub syms: Vec<Keysym>,
}

impl KeyMap {
    pub(super) fn read<C: Connection>(conn: &C) -> io::Result<Self> {
        let setup = conn.setup();
        let min = setup.min_keycode;
        let count = setup.max_keycode - min + 1;
        let reply = conn
            .get_keyboard_mapping(min, count)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        Ok(Self {
            min,
            per_code: reply.keysyms_per_keycode,
            syms: reply.keysyms,
        })
    }

    fn row(&self, index: usize) -> &[Keysym] {
        let per = usize::from(self.per_code);
        &self.syms[index * per..(index + 1) * per]
    }

    fn codes(&self) -> usize {
        if self.per_code == 0 {
            0
        } else {
            self.syms.len() / usize::from(self.per_code)
        }
    }

    /// The key of `sym` and whether Shift is needed.
    pub(super) fn find(&self, sym: Keysym) -> Option<(Keycode, bool)> {
        (0..self.codes()).find_map(|index| {
            let row = self.row(index);
            let code = self.min + index as u8;
            if row.first() == Some(&sym) {
                Some((code, false))
            } else if row.get(1) == Some(&sym) {
                Some((code, true))
            } else {
                None
            }
        })
    }

    /// The highest keycode without any keysym.
    pub(super) fn spare(&self) -> Option<Keycode> {
        (0..self.codes())
            .rev()
            .find(|index| self.row(*index).iter().all(|sym| *sym == NO_SYMBOL))
            .map(|index| self.min + index as u8)
    }
}

fn press<C: Connection>(conn: &C, root: Window, code: Keycode, down: bool) -> io::Result<()> {
    let kind = if down { KEY_PRESS } else { KEY_RELEASE };
    conn.xtest_fake_input(kind, code, 0, root, 0, 0, 0)
        .map_err(io::Error::other)?;
    Ok(())
}

fn tap<C: Connection>(conn: &C, root: Window, code: Keycode, shift: bool) -> io::Result<()> {
    if shift {
        press(conn, root, SHIFT_KEYCODE, true)?;
    }
    press(conn, root, code, true)?;
    press(conn, root, code, false)?;
    if shift {
        press(conn, root, SHIFT_KEYCODE, false)?;
    }
    Ok(())
}

/// Waits until the server handled every request so far.
fn sync<C: Connection>(conn: &C) -> io::Result<()> {
    conn.get_input_focus()
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    Ok(())
}

/// Types `text`; returns how many characters had no key at all.
pub(super) fn type_text<C: Connection>(conn: &C, root: Window, text: &str) -> io::Result<usize> {
    let mut map = KeyMap::read(conn)?;
    let mut skipped = 0;
    for character in text.chars() {
        let sym = keysym_of(character);
        if let Some((code, shift)) = map.find(sym) {
            tap(conn, root, code, shift)?;
            continue;
        }
        let Some(spare) = map.spare() else {
            skipped += 1;
            continue;
        };
        let per = map.per_code.max(1);
        let column = vec![sym; usize::from(per)];
        conn.change_keyboard_mapping(1, spare, per, &column)
            .map_err(io::Error::other)?;
        sync(conn)?;
        tap(conn, root, spare, false)?;
        sync(conn)?;
        // Clients read the mapping lazily: give them the press first.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let empty = vec![NO_SYMBOL; usize::from(per)];
        conn.change_keyboard_mapping(1, spare, per, &empty)
            .map_err(io::Error::other)?;
        sync(conn)?;
        map = KeyMap::read(conn)?;
    }
    Ok(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_x11_keysym_and_keymap_lookup() {
        assert_eq!(keysym_of('a'), 0x61);
        assert_eq!(keysym_of('ä'), 0xE4);
        assert_eq!(keysym_of('€'), 0x0100_20AC);
        assert_eq!(keysym_of('\n'), 0xFF0D);
        let map = KeyMap {
            min: 8,
            per_code: 2,
            syms: vec![0, 0, 0x61, 0x41, 0x31, 0x21, 0, 0],
        };
        assert_eq!(map.find(0x61), Some((9, false)));
        assert_eq!(map.find(0x21), Some((10, true)));
        assert_eq!(map.find(0xE4), None);
        assert_eq!(map.spare(), Some(11));
    }
}
