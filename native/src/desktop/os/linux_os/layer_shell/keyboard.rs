//! The keyboard of the layer backend (DSK-M7, U37-11). `wl_keyboard` gives
//! the keymap as a file descriptor, the modifiers, the focus and the keys;
//! xkbcommon (loaded at run time, `xkb.rs`) turns keys into keysyms and
//! text, `keysyms.rs` into egui events. Only a layer surface with the
//! keyboard interactivity `on_demand` (protocol version 4) gets a keyboard;
//! older compositors have none. Holding a key repeats it at the compositor's
//! rate after its delay. There are no input methods (`DESKTOP-LAYER-IME`).
use std::time::{Duration, Instant};

use eframe::egui::Event;
use wayland_client::protocol::wl_keyboard::{self, KeyState, KeymapFormat, WlKeyboard};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

use super::keysyms::{key_events, KeyPress};
use super::state::LayerState;
use super::xkb::Xkb;

/// Repeat rate (keys per second) and delay without `repeat_info`
/// (keyboard version below 4): the Wayland convention.
const DEFAULT_RATE: i32 = 25;
const DEFAULT_DELAY_MS: i32 = 600;

/// Suite switch (DSK-M15): every key the surface receives is appended to this
/// file as `press|release 0x<keysym>`; never set in production.
const TEST_KEYLOG: &str = "SE_DESKTOP_TEST_KEYLOG";

fn log_key(pressed: bool, sym: u32) {
    use std::io::Write;
    let Some(path) = std::env::var_os(TEST_KEYLOG).filter(|path| !path.is_empty()) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            file,
            "{} {sym:#x}",
            if pressed { "press" } else { "release" }
        );
    }
}

/// The key held down and when it repeats next.
struct Held {
    key: u32,
    next: Instant,
}

pub(super) struct KeyboardEntry {
    keyboard: WlKeyboard,
    pub(super) seat: WlSeat,
    xkb: Xkb,
    focus: Option<WlSurface>,
    rate: i32,
    delay_ms: i32,
    held: Option<Held>,
}

impl KeyboardEntry {
    pub(super) fn new(keyboard: WlKeyboard, seat: WlSeat, xkb: Xkb) -> KeyboardEntry {
        KeyboardEntry {
            keyboard,
            seat,
            xkb,
            focus: None,
            rate: DEFAULT_RATE,
            delay_ms: DEFAULT_DELAY_MS,
            held: None,
        }
    }

    pub(super) fn held_next(&self) -> Option<Instant> {
        self.held.as_ref().map(|held| held.next)
    }

    pub(super) fn release(&self) {
        if self.keyboard.version() >= 3 {
            self.keyboard.release();
        }
    }
}

impl LayerState {
    fn keyboard_index(&self, keyboard: &WlKeyboard) -> Option<usize> {
        self.input
            .keyboards
            .iter()
            .position(|entry| &entry.keyboard == keyboard)
    }

    /// The text for egui's `Paste`: the clipboard's text, empty without one.
    fn paste_text(&self) -> String {
        self.clipboard
            .as_ref()
            .map(|clipboard| clipboard.paste_text())
            .unwrap_or_default()
    }

    fn set_focus(&mut self, surface: &WlSurface, focused: bool) {
        if let Some(view) = self.view_for(surface) {
            view.frame.focused = focused;
            view.frame.dirty = true;
            view.push([Event::WindowFocused(focused)]);
        }
    }

    fn key_event(&mut self, index: usize, serial: u32, key: u32, pressed: bool) {
        let entry = &mut self.input.keyboards[index];
        let seat = entry.seat.clone();
        let Some(surface) = entry.focus.clone() else {
            return;
        };
        let modifiers = entry.xkb.modifiers();
        let sym = entry.xkb.sym(key);
        let text = if pressed { entry.xkb.text(key) } else { None };
        if pressed && entry.rate > 0 && entry.xkb.repeats(key) {
            let delay = Duration::from_millis(entry.delay_ms.max(0) as u64);
            entry.held = Some(Held {
                key,
                next: Instant::now() + delay,
            });
        } else if !pressed && entry.held.as_ref().is_some_and(|held| held.key == key) {
            entry.held = None;
        }
        self.input.modifiers = modifiers;
        log_key(pressed, sym);
        if let Some(clipboard) = &self.clipboard {
            clipboard.note_serial(&seat, serial);
        }
        let press = KeyPress {
            sym,
            text: text.as_deref(),
            pressed,
            repeat: false,
            modifiers,
        };
        let events = key_events(press, || self.paste_text());
        self.push_events(&surface, events);
    }

    /// Repeats the held keys that are due; the next repeat follows after one
    /// period.
    pub(super) fn tick_repeat(&mut self, now: Instant) {
        for index in 0..self.input.keyboards.len() {
            let entry = &mut self.input.keyboards[index];
            let Some(held) = entry.held.as_mut().filter(|held| held.next <= now) else {
                continue;
            };
            let key = held.key;
            let period = Duration::from_secs_f64(1.0 / f64::from(entry.rate.max(1)));
            held.next = now + period;
            let Some(surface) = entry.focus.clone() else {
                entry.held = None;
                continue;
            };
            let modifiers = entry.xkb.modifiers();
            let sym = entry.xkb.sym(key);
            let text = entry.xkb.text(key);
            let press = KeyPress {
                sym,
                text: text.as_deref(),
                pressed: true,
                repeat: true,
                modifiers,
            };
            let events = key_events(press, String::new);
            self.push_events(&surface, events);
        }
    }
}

impl Dispatch<WlKeyboard, ()> for LayerState {
    fn event(
        state: &mut Self,
        keyboard: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.keyboard_index(keyboard) else {
            return;
        };
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(KeymapFormat::XkbV1)
                    && !state.input.keyboards[index]
                        .xkb
                        .set_keymap(fd, size as usize)
                {
                    eprintln!("se Desktop: Layer-Backend: Tastaturbelegung nicht lesbar");
                }
            }
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                let seat = state.input.keyboards[index].seat.clone();
                if let Some(clipboard) = &state.clipboard {
                    clipboard.note_serial(&seat, serial);
                }
                state.input.keyboards[index].focus = Some(surface.clone());
                state.set_focus(&surface, true);
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let entry = &mut state.input.keyboards[index];
                entry.held = None;
                entry.focus = None;
                state.set_focus(&surface, false);
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(KeyState::Pressed));
                state.key_event(index, serial, key, pressed);
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let entry = &mut state.input.keyboards[index];
                entry
                    .xkb
                    .update_mask(mods_depressed, mods_latched, mods_locked, group);
                state.input.modifiers = entry.xkb.modifiers();
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                let entry = &mut state.input.keyboards[index];
                entry.rate = rate;
                entry.delay_ms = delay;
            }
            _ => {}
        }
    }
}
