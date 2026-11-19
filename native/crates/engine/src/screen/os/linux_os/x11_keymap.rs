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
        '\n' | '\r' => 0xFF0D, // Return
        '\t' => 0xFF09,        // Tab
        '\u{8}' => 0xFF08,     // BackSpace
        ' '..='~' | '\u{A0}'..='\u{FF}' => character as u32,
        // The layouts bind the euro sign as `EuroSign` (Gemini 3/11).
        '€' => 0x20AC,
        other => 0x0100_0000 + other as u32,
    }
}

/// The characters of `text` as they are typed: a Windows line end
/// (`\r\n`) is one Return, not two (review 2026-11-19, Gemini 12.3: the
/// `\r` went out as the Unicode keysym U+000D, which the RemoteDesktop
/// portal may refuse).
pub(super) fn typed_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || loop {
        let character = chars.next()?;
        if character == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        return Some(character);
    })
}

/// `ISO_Level3_Shift` (AltGr).
const LEVEL3_SHIFT: Keysym = 0xFE03;

/// The keyboard mapping as the server holds it now.
pub(super) struct KeyMap {
    pub min: Keycode,
    pub per_code: u8,
    pub syms: Vec<Keysym>,
    /// The AltGr key, when one carries `ISO_Level3_Shift` and is a modifier:
    /// the third and fourth level (columns 4 and 5) are typed with it
    /// (Gemini 3/11: `@`, `{`, `€` … on a German layout took the spare key).
    pub level3: Option<Keycode>,
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
        let mut map = Self {
            min,
            per_code: reply.keysyms_per_keycode,
            syms: reply.keysyms,
            level3: None,
        };
        let modifiers = conn
            .get_modifier_mapping()
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        map.level3 = map
            .find(LEVEL3_SHIFT)
            .filter(|(code, shift, _)| !shift && modifiers.keycodes.contains(code))
            .map(|(code, _, _)| code);
        Ok(map)
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

    /// The key of `sym`, whether Shift and whether AltGr is needed; the
    /// first two levels before the third and fourth.
    pub(super) fn find(&self, sym: Keysym) -> Option<(Keycode, bool, bool)> {
        let levels: &[(usize, bool, bool)] = if self.level3.is_some() {
            &[
                (0, false, false),
                (1, true, false),
                (4, false, true),
                (5, true, true),
            ]
        } else {
            &[(0, false, false), (1, true, false)]
        };
        levels.iter().find_map(|(column, shift, level3)| {
            (0..self.codes()).find_map(|index| {
                (self.row(index).get(*column) == Some(&sym)).then_some((
                    self.min + index as u8,
                    *shift,
                    *level3,
                ))
            })
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

/// `Shift_R` as a Linux code (54) plus the evdev offset 8.
const SHIFT_R_KEYCODE: Keycode = 62;

/// The keys down on the server now (`QueryKeymap`): the viewer may hold a
/// modifier the text must not let go of (Codex 5/15).
fn held_keys<C: Connection>(conn: &C) -> io::Result<Vec<Keycode>> {
    let keys = conn
        .query_keymap()
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?
        .keys;
    Ok((0..=255u8)
        .filter(|code| keys[usize::from(code / 8)] & (1 << (code % 8)) != 0)
        .collect())
}

/// Presses `code` at its level: `Some((shift, level3))`, or `None` for any
/// level (a spare key carries the keysym in every column); `altgr` is the
/// map's AltGr key. A Shift or AltGr the viewer holds (`held`) stays as it
/// is when the level needs it; one the level must not have is lifted for
/// the press and pressed again after (Codex 5/15: the text let go of a
/// held Shift).
fn tap<C: Connection>(
    conn: &C,
    root: Window,
    code: Keycode,
    level: Option<(bool, bool)>,
    altgr: Option<Keycode>,
    held: &[Keycode],
) -> io::Result<()> {
    let mut lift: Vec<Keycode> = Vec::new();
    let (mut press_shift, mut press_altgr) = (false, None);
    if let Some((shift, level3)) = level {
        let shifts = [SHIFT_KEYCODE, SHIFT_R_KEYCODE];
        if shift {
            press_shift = !shifts.iter().any(|key| held.contains(key));
        } else {
            lift.extend(shifts.into_iter().filter(|key| held.contains(key)));
        }
        match altgr {
            Some(key) if level3 && !held.contains(&key) => press_altgr = Some(key),
            Some(key) if !level3 && held.contains(&key) => lift.push(key),
            _ => {}
        }
    }
    for key in &lift {
        press(conn, root, *key, false)?;
    }
    if let Some(key) = press_altgr {
        press(conn, root, key, true)?;
    }
    if press_shift {
        press(conn, root, SHIFT_KEYCODE, true)?;
    }
    press(conn, root, code, true)?;
    press(conn, root, code, false)?;
    if press_shift {
        press(conn, root, SHIFT_KEYCODE, false)?;
    }
    if let Some(key) = press_altgr {
        press(conn, root, key, false)?;
    }
    for key in &lift {
        press(conn, root, *key, true)?;
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

/// Codex 21 (review 2026-11-19): whether every character of `texts` gets a
/// key: one of the mapping, or the spare keycode `type_text` remaps for the
/// others. A text that cannot go whole is refused before any of it goes.
pub(super) fn typable<'a, C: Connection>(
    conn: &C,
    mut texts: impl Iterator<Item = &'a str>,
) -> io::Result<bool> {
    let map = KeyMap::read(conn)?;
    Ok(map.spare().is_some()
        || texts.all(|text| typed_chars(text).all(|c| map.find(keysym_of(c)).is_some())))
}

/// Types `text`; returns how many characters had no key at all.
pub(super) fn type_text<C: Connection>(conn: &C, root: Window, text: &str) -> io::Result<usize> {
    let mut map = KeyMap::read(conn)?;
    let held = held_keys(conn)?;
    let mut skipped = 0;
    for character in typed_chars(text) {
        let sym = keysym_of(character);
        if let Some((code, shift, level3)) = map.find(sym) {
            tap(conn, root, code, Some((shift, level3)), map.level3, &held)?;
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
        tap(conn, root, spare, None, None, &held)?;
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
        assert_eq!(keysym_of('€'), 0x20AC);
        assert_eq!(keysym_of('\n'), 0xFF0D);
        assert_eq!(keysym_of('\r'), 0xFF0D);
        assert_eq!(typed_chars("a\r\nb\rc").collect::<String>(), "a\nb\rc");
        let map = KeyMap {
            min: 8,
            per_code: 2,
            syms: vec![0, 0, 0x61, 0x41, 0x31, 0x21, 0, 0],
            level3: None,
        };
        assert_eq!(map.find(0x61), Some((9, false, false)));
        assert_eq!(map.find(0x21), Some((10, true, false)));
        assert_eq!(map.find(0xE4), None);
        assert_eq!(map.spare(), Some(11));
        // Gemini 3/11: with an AltGr key the third and fourth levels count
        // (German `q`: q Q @ … in columns 0, 1, 4), the first two first.
        let german = KeyMap {
            min: 8,
            per_code: 6,
            syms: vec![
                0x71, 0x51, 0x71, 0x51, 0x40, 0x0FD9, // q
                0x65, 0x45, 0x65, 0x45, 0x20AC, 0, // e
                0, 0, 0, 0, 0, 0, // spare
            ],
            level3: Some(92),
        };
        assert_eq!(german.find(0x40), Some((8, false, true)));
        assert_eq!(german.find(0x20AC), Some((9, false, true)));
        assert_eq!(german.find(0x0FD9), Some((8, true, true)));
        assert_eq!(german.find(0x51), Some((8, true, false)));
        let plain = KeyMap {
            level3: None,
            ..german
        };
        assert_eq!(plain.find(0x40), None);
    }
}
