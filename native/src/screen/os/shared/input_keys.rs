//! Key codes of the desktop hosts' input (SCR-M6): the HID usage of an
//! `InputEvent::Key` as the Linux input event code (`KEY_*`, the code of the
//! RemoteDesktop portal and, plus 8, the X11 keycode of an evdev keyboard)
//! and as the PC scan code `SendInput` takes on Windows. Plain tables, no
//! platform code; the HID → `KEY_*` pairs follow the Linux HID keyboard
//! table, the scan codes the PC/AT set 1 with its `E0` prefix.
use crate::screen::core::input_model::CONSUMER_PAGE;

/// The Linux `KEY_*` code of HID usage `usage`; `None` for a key the hosts
/// do not press.
pub(crate) fn evdev_code(usage: u16) -> Option<u16> {
    #[rustfmt::skip]
    const LETTERS: [u16; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47,
        17, 45, 21, 44,
    ];
    #[rustfmt::skip]
    const FROM_ENTER: [u16; 0x66 - 0x28] = [
        28, 1, 14, 15, 57, 12, 13, 26, 27, 43, 43, 39, 40, 41, 51, 52, 53, 58, // 0x28–0x39
        59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88, // F1–F12, 0x3A–0x45
        99, 70, 119, 110, 102, 104, 111, 107, 109, 106, 105, 108, 103, 69, // 0x46–0x53
        98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71, 72, 73, 82, 83, // keypad 0x54–0x63
        86, 127, // 0x64 intl backslash, 0x65 menu
    ];
    match usage {
        0x04..=0x1D => Some(LETTERS[usize::from(usage - 0x04)]),
        0x1E..=0x26 => Some(usage - 0x1E + 2),
        0x27 => Some(11),
        0x28..=0x65 => Some(FROM_ENTER[usize::from(usage - 0x28)]),
        0x68..=0x73 => Some(usage - 0x68 + 183),
        0x7B => Some(137),
        0x7C => Some(133),
        0x7D => Some(135),
        0xE0 => Some(29),
        0xE1 => Some(42),
        0xE2 => Some(56),
        0xE3 => Some(125),
        0xE4 => Some(97),
        0xE5 => Some(54),
        0xE6 => Some(100),
        0xE7 => Some(126),
        _ if usage == CONSUMER_PAGE | 0xCD => Some(164),
        _ if usage == CONSUMER_PAGE | 0xE2 => Some(113),
        _ if usage == CONSUMER_PAGE | 0xE9 => Some(115),
        _ if usage == CONSUMER_PAGE | 0xEA => Some(114),
        _ if usage == CONSUMER_PAGE | 0x224 => Some(158),
        _ => None,
    }
}

/// A key as Windows takes it: a set-1 scan code (with the `E0` prefix as
/// `extended`) or, for keys without a plain scan code, a virtual key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WinKey {
    Scan { code: u16, extended: bool },
    Virtual(u16),
}

/// The Windows key of HID usage `usage`.
pub(crate) fn windows_key(usage: u16) -> Option<WinKey> {
    let code = evdev_code(usage)?;
    let scan = |code: u16, extended: bool| Some(WinKey::Scan { code, extended });
    match code {
        // Below 89 the Linux code is the set-1 scan code.
        1..=88 => scan(code, false),
        96 => scan(0x1C, true),                      // keypad Enter
        97 => scan(0x1D, true),                      // right Ctrl
        98 => scan(0x35, true),                      // keypad /
        99 => scan(0x37, true),                      // Print Screen
        100 => scan(0x38, true),                     // right Alt
        102 => scan(0x47, true),                     // Home
        103 => scan(0x48, true),                     // Up
        104 => scan(0x49, true),                     // Page Up
        105 => scan(0x4B, true),                     // Left
        106 => scan(0x4D, true),                     // Right
        107 => scan(0x4F, true),                     // End
        108 => scan(0x50, true),                     // Down
        109 => scan(0x51, true),                     // Page Down
        110 => scan(0x52, true),                     // Insert
        111 => scan(0x53, true),                     // Delete
        125 => scan(0x5B, true),                     // left Windows
        126 => scan(0x5C, true),                     // right Windows
        127 => scan(0x5D, true),                     // Menu
        183..=193 => scan(code - 183 + 0x64, false), // F13–F23
        194 => scan(0x76, false),                    // F24
        119 => Some(WinKey::Virtual(0x13)),          // Pause (VK_PAUSE)
        164 => Some(WinKey::Virtual(0xB3)),          // VK_MEDIA_PLAY_PAUSE
        113 => Some(WinKey::Virtual(0xAD)),          // VK_VOLUME_MUTE
        115 => Some(WinKey::Virtual(0xAF)),          // VK_VOLUME_UP
        114 => Some(WinKey::Virtual(0xAE)),          // VK_VOLUME_DOWN
        158 => Some(WinKey::Virtual(0xA6)),          // VK_BROWSER_BACK
        _ => None,
    }
}

/// The keys of the viewer's toolbar the desktop hosts press (C5).
pub(crate) fn toolbar_keys() -> Vec<u16> {
    crate::screen::core::input_model::TOOLBAR_KEYS
        .iter()
        .copied()
        .filter(|usage| evdev_code(*usage).is_some())
        .collect()
}

/// Ctrl+Alt+Delete is never sent (`no_ctrl_alt_del`): the Delete press
/// while both a Ctrl and an Alt key are held is dropped.
#[derive(Debug, Default)]
pub(crate) struct SasGuard {
    ctrl: u8,
    alt: u8,
}

impl SasGuard {
    pub(crate) const fn new() -> Self {
        Self { ctrl: 0, alt: 0 }
    }

    /// Follows `usage` going `down`; `false` for the press to drop.
    pub(crate) fn allows(&mut self, usage: u16, down: bool) -> bool {
        let step = |count: &mut u8| {
            *count = if down {
                count.saturating_add(1)
            } else {
                count.saturating_sub(1)
            }
        };
        match usage {
            0xE0 | 0xE4 => step(&mut self.ctrl),
            0xE2 | 0xE6 => step(&mut self.alt),
            0x4C | 0x63 => return !(down && self.ctrl > 0 && self.alt > 0),
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_input_keys_tables_and_no_ctrl_alt_del() {
        assert_eq!(evdev_code(0x04), Some(30)); // a
        assert_eq!(evdev_code(0x1D), Some(44)); // z
        assert_eq!(evdev_code(0x27), Some(11)); // 0
        assert_eq!(evdev_code(0x45), Some(88)); // F12
        assert_eq!(evdev_code(0x52), Some(103)); // Up
        assert_eq!(evdev_code(0x63), Some(83)); // keypad .
        assert_eq!(evdev_code(0x68), Some(183)); // F13
        assert_eq!(evdev_code(0x02), None);
        assert_eq!(
            windows_key(0x04),
            Some(WinKey::Scan {
                code: 0x1E,
                extended: false
            })
        );
        assert_eq!(
            windows_key(0x4F),
            Some(WinKey::Scan {
                code: 0x4D,
                extended: true
            })
        );
        assert_eq!(windows_key(0x48), Some(WinKey::Virtual(0x13)));
        assert_eq!(toolbar_keys().len(), 11);
        let mut guard = SasGuard::default();
        assert!(guard.allows(0xE0, true));
        assert!(guard.allows(0xE2, true));
        assert!(!guard.allows(0x4C, true), "Ctrl+Alt+Delete must be dropped");
        assert!(guard.allows(0x4C, false));
        assert!(guard.allows(0xE2, false));
        assert!(guard.allows(0x4C, true), "Ctrl+Delete stays allowed");
    }

    fn control_session(device: &str) -> crate::screen::core::registry::NewSession {
        use crate::screen::core::session_state::SessionState;
        use crate::screen::core::wire_hello::SessionMode;
        crate::screen::core::registry::NewSession {
            principal: crate::share::relation_proof::ProvenPrincipal {
                relation_kind: "direct".into(),
                relation_id: format!("relation-{device}"),
                device_id: device.into(),
                device_name: device.into(),
                public_key: format!("key-{device}"),
                fingerprint: format!("fp-{device}"),
                node_id: format!("key-{device}"),
            },
            device_name: device.to_uppercase(),
            mode: SessionMode::Control,
            source: "monitor:1".into(),
            path: "direct".into(),
            started_at: 1_760_000_000,
            state: SessionState::Starting,
            screen_content: true,
        }
    }

    /// One viewer controls a host at a time (C5); the next takes over once
    /// the first gave control back or went.
    #[test]
    fn screen_task_control_exclusive() {
        use crate::screen::core::registry::ScreenRegistry;
        let registry = std::sync::Arc::new(ScreenRegistry::default());
        let first = registry.register(control_session("laptop")).unwrap();
        let second = registry.register(control_session("tablet")).unwrap();
        first.take_control().unwrap();
        assert_eq!(second.take_control().unwrap_err().device_id, "laptop");
        assert!(first.release_control());
        second.take_control().unwrap();
        assert_eq!(first.take_control().unwrap_err().device_id, "tablet");
        drop(second);
        first.take_control().unwrap();
        assert_eq!(registry.controller().unwrap().device_id, "laptop");
    }

    /// What the host pressed for the viewer goes up when the session ends,
    /// gives control back or loses its connection; modifiers last.
    #[test]
    fn screen_task_keys_released_on_disconnect() {
        use crate::screen::core::input_model::{
            InputEvent, PointerButton, PressedInputs, TouchPhase,
        };
        let mut pressed = PressedInputs::default();
        for event in [
            InputEvent::Key {
                hid_usage: 0xE0,
                down: true,
            },
            InputEvent::Key {
                hid_usage: 0x04,
                down: true,
            },
            InputEvent::Button {
                button: PointerButton::Left,
                down: true,
            },
            InputEvent::Touch {
                id: 0,
                phase: TouchPhase::Start,
                x: 10,
                y: 10,
            },
        ] {
            pressed.observe(&event);
        }
        let released = pressed.release_all();
        assert_eq!(released.len(), 4);
        assert!(matches!(
            released[0],
            InputEvent::Touch {
                phase: TouchPhase::Cancel,
                ..
            }
        ));
        assert_eq!(
            released[3],
            InputEvent::Key {
                hid_usage: 0xE0,
                down: false
            }
        );
        assert!(pressed.is_empty());
        // Every released key has a code on both desktop platforms.
        for event in &released {
            if let InputEvent::Key { hid_usage, .. } = event {
                assert!(evdev_code(*hid_usage).is_some() && windows_key(*hid_usage).is_some());
            }
        }
    }
}
