//! The input of the desktop viewer (SCR-M7, U25-8): egui events become the
//! `InputEvent`s of the `INPUT` frame. The pointer is mapped to 0–65535
//! across the picture, keys become HID usages (`input_model::key_usage`),
//! modifiers the left keys (egui reports no right ones), typing and IME
//! commits `Text` (the host's layout decides the character), shortcuts with
//! Ctrl or Alt keys (winit leaves Ctrl/Alt out of AltGr: its text is `Text`).
//! Every press is tracked (`PressedInputs`) for `release_all`; one frame's
//! events go out in batches (`take_batch`), the rest with the next frame.
use std::collections::VecDeque;

use eframe::egui::{
    Event, ImeEvent, Key, Modifiers, MouseWheelUnit, PointerButton as EguiButton, Pos2,
    TouchPhase as EguiPhase,
};

use super::screen_input_keys::{is_text_key, key_usage_of, local_key};
use crate::screen::core::input_model::{
    key_usage, InputCapabilities, InputEvent, PointerButton, PressedInputs, TouchPhase,
    MAX_INPUT_EVENTS, MAX_TEXT_BYTES,
};
use crate::screen::core::wire::MAX_CONTROL_BYTES;
#[cfg(test)]
use crate::screen::core::{input_model::pixel_to_pointer, pixel_convert::FrameSize};

/// Wheel notches per line of a mouse wheel in lines (1/120 notch units).
const NOTCH: f32 = 120.0;
/// Points of a touchpad scroll that make one notch.
const POINTS_PER_NOTCH: f32 = 40.0;
/// Lines a page scroll counts for.
const LINES_PER_PAGE: f32 = 10.0;
/// Room kept free in a batch for its count and the largest fixed event.
const BATCH_MARGIN: usize = 16;

pub use super::screen_viewport::Geometry;

fn button_of(button: EguiButton) -> PointerButton {
    match button {
        EguiButton::Primary => PointerButton::Left,
        EguiButton::Secondary => PointerButton::Right,
        EguiButton::Middle => PointerButton::Middle,
        EguiButton::Extra1 => PointerButton::Back,
        EguiButton::Extra2 => PointerButton::Forward,
    }
}

fn phase_of(phase: EguiPhase) -> TouchPhase {
    match phase {
        EguiPhase::Start => TouchPhase::Start,
        EguiPhase::Move => TouchPhase::Move,
        EguiPhase::End => TouchPhase::End,
        EguiPhase::Cancel => TouchPhase::Cancel,
    }
}

/// Bytes an event takes in an `INPUT` frame (an upper bound).
fn event_bytes(event: &InputEvent) -> usize {
    match event {
        InputEvent::Text { text } => 3 + text.len(),
        _ => 7,
    }
}

/// Turns the egui events of each frame into input for the host.
pub struct InputMapper {
    caps: InputCapabilities,
    mods: Modifiers,
    pressed: PressedInputs,
    held_buttons: Vec<PointerButton>,
    backlog: VecDeque<InputEvent>,
    pending_move: Option<(u16, u16)>,
    last_sent: Option<(u16, u16)>,
    hover_inside: bool,
    /// Keys the viewer kept for itself: their release is not sent either.
    swallowed: Vec<Key>,
    suppress_modifiers: bool,
    /// Keys the host receives as `Text`: neither press nor release is sent.
    text_keys: Vec<Key>,
    /// Usages whose press went to the host: only their release follows
    /// (Codex 4/7: an Enter that confirmed a toolbar field sent its release).
    keys_down: Vec<u16>,
    /// No frame went through since the mapper began or let everything go:
    /// modifiers already held then belong to the local user (H5).
    fresh: bool,
    /// The modifiers held in the frames before (`note_local`): only these
    /// stay local in the first frame (K1).
    armed: Modifiers,
    wheel_rest: (f32, f32),
    touch_active: bool,
    /// Ctrl+V or Umschalt+Einfg watched for a swallowed paste (`missed_paste`).
    paste: paste::PasteWatch,
    /// Keys without a usage that were dropped (shown in the statistics).
    pub unmapped_keys: u64,
    /// The text of a running IME composition (shown, not sent).
    pub preedit: Option<String>,
}

impl InputMapper {
    pub fn new(caps: InputCapabilities) -> Self {
        Self {
            caps,
            mods: Modifiers::NONE,
            pressed: PressedInputs::default(),
            held_buttons: Vec::new(),
            backlog: VecDeque::new(),
            pending_move: None,
            last_sent: None,
            hover_inside: false,
            swallowed: Vec::new(),
            suppress_modifiers: false,
            text_keys: Vec::new(),
            keys_down: Vec::new(),
            fresh: true,
            // Made in its first frame: what is held came before it.
            armed: Modifiers {
                ctrl: true,
                shift: true,
                alt: true,
                ..Modifiers::NONE
            },
            wheel_rest: (0.0, 0.0),
            touch_active: false,
            paste: paste::PasteWatch::default(),
            unmapped_keys: 0,
            preedit: None,
        }
    }

    /// A frame in which the host gets no keys: the modifiers held then
    /// belong to the local user (K1, `fresh`).
    pub fn note_local(&mut self, mods: Modifiers) {
        if self.fresh {
            self.armed = mods;
        }
    }

    pub fn has_backlog(&self) -> bool {
        !self.backlog.is_empty()
    }

    /// Everything the host holds down for this viewer is lifted.
    pub fn release_all(&mut self) {
        self.flush_move();
        let released = self.pressed.release_all();
        self.backlog.extend(released);
        self.held_buttons.clear();
        self.text_keys.clear();
        self.keys_down.clear();
        self.fresh = true;
        self.armed = Modifiers::NONE;
        // Modifiers still held locally are kept from the host until they are
        // let go, but only when the host had one down (Gemini 3/23: the
        // first shortcut after any release was swallowed).
        self.suppress_modifiers = self.mods.ctrl || self.mods.alt || self.mods.shift;
        self.mods = Modifiers::NONE;
        self.touch_active = false;
    }

    /// The next batch for one `INPUT` frame: at most `MAX_INPUT_EVENTS`
    /// events and what the frame takes.
    pub fn take_batch(&mut self) -> Option<Vec<InputEvent>> {
        let mut batch = Vec::new();
        let mut bytes = 2usize;
        while let Some(next) = self.backlog.front() {
            let size = event_bytes(next);
            let full =
                batch.len() >= MAX_INPUT_EVENTS || bytes + size > MAX_CONTROL_BYTES - BATCH_MARGIN;
            if !batch.is_empty() && full {
                break;
            }
            bytes += size;
            if let Some(event) = self.backlog.pop_front() {
                batch.push(event);
            }
        }
        (!batch.is_empty()).then_some(batch)
    }

    /// Maps the events of one frame; `frame_mods` are the modifiers at its end.
    pub fn process(&mut self, events: &[Event], frame_mods: Modifiers, geometry: &Geometry) {
        if events.iter().any(|event| matches!(event, Event::Key {
            key, modifiers, pressed: true, ..
        } if matches!(local_key(*key, *modifiers), Some(super::screen_input_keys::LocalKey::Fullscreen | super::screen_input_keys::LocalKey::ExitFullscreen)))) {
            self.release_all();
            self.suppress_modifiers = true;
            for event in events {
                if let Event::Key { key, modifiers, pressed:true, .. } = event {
                    if local_key(*key,*modifiers).is_some() && !self.swallowed.contains(key) { self.swallowed.push(*key); }
                }
            }
            return;
        }
        // Modifiers held from before stay local until let go (H5, K1); a
        // new press (no repeat, round 6) or a chord is input for the host.
        let pressing = events.iter().any(|event| {
            matches!(
                event,
                Event::Key {
                    pressed: true,
                    repeat: false,
                    ..
                } | Event::PointerButton { pressed: true, .. }
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
            )
        });
        if events.contains(&Event::WindowFocused(true)) {
            (self.fresh, self.armed) = (true, frame_mods); // N3: Alt+Tab back
        }
        // A frame without modifiers and presses decides nothing yet.
        let any = frame_mods.ctrl || frame_mods.alt || frame_mods.shift;
        if self.fresh && (any || pressing) {
            self.fresh = false;
            let armed = self.armed;
            let held = (frame_mods.ctrl && armed.ctrl)
                || (frame_mods.alt && armed.alt)
                || (frame_mods.shift && armed.shift);
            self.suppress_modifiers |= held && !pressing;
        }
        if self.suppress_modifiers {
            if frame_mods.ctrl || frame_mods.alt || frame_mods.shift {
                for event in events {
                    if let Event::Key {
                        key,
                        pressed: false,
                        ..
                    } = event
                    {
                        self.swallowed.retain(|held| held != key);
                    }
                }
                return;
            }
            self.suppress_modifiers = false;
        }
        // egui reports a touch as a touch and as pointer events; the host
        // that takes touches gets the touch only.
        let touching = self.caps.touch
            && events.iter().any(|event| {
                matches!(
                    event,
                    Event::Touch {
                        phase: EguiPhase::Start | EguiPhase::Move,
                        ..
                    }
                )
            });
        if touching {
            self.touch_active = true;
        }
        // Gemini 8/12: behind the focus and held-modifier gates, so a paste
        // pressed in another window is no input here.
        self.missed_paste(events, frame_mods);
        for event in events {
            self.event(event, geometry);
        }
        self.sync_modifiers(frame_mods);
        self.flush_move();
    }

    fn event(&mut self, event: &Event, geometry: &Geometry) {
        match event {
            Event::PointerMoved(pos) => self.pointer_moved(*pos, geometry),
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                self.sync_modifiers(*modifiers);
                self.pointer_button(*pos, *button, *pressed, geometry);
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
            } => {
                self.sync_modifiers(*modifiers);
                self.wheel(*unit, delta.x, delta.y);
            }
            Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } => self.key_event(*key, *physical_key, *pressed, *repeat, *modifiers),
            Event::Text(text) => {
                if self.caps.text && !self.mods.ctrl && !self.mods.alt {
                    self.text(text);
                }
            }
            Event::Ime(ime) => self.ime(ime),
            Event::Copy => self.copy(),
            Event::Cut => self.cut(),
            Event::Paste(text) => self.paste(text),
            Event::Touch { id, phase, pos, .. } => self.touch(id.0, *phase, *pos, geometry),
            Event::WindowFocused(false) => self.release_all(),
            _ => {}
        }
    }

    fn emit(&mut self, event: InputEvent) {
        self.flush_move();
        self.pressed.observe(&event);
        self.backlog.push_back(event);
    }

    fn flush_move(&mut self) {
        if let Some((x, y)) = self.pending_move.take() {
            if self.last_sent != Some((x, y)) {
                self.last_sent = Some((x, y));
                self.backlog.push_back(InputEvent::PointerAbs { x, y });
            }
        }
    }

    /// Sends the modifier keys that changed since the last event.
    fn sync_modifiers(&mut self, now: Modifiers) {
        let changes = [
            (now.ctrl, self.mods.ctrl, "ctrl_left"),
            (now.shift, self.mods.shift, "shift_left"),
            (now.alt, self.mods.alt, "alt_left"),
        ];
        // Releases first and Alt before Ctrl, presses Ctrl before Alt (round
        // 9 D7: an Alt let go alone after Ctrl can open a Windows menu).
        let releases = changes.iter().rev().filter(|(down, was, _)| !down && *was);
        let presses = changes.iter().filter(|(down, was, _)| *down && !was);
        for (down, _, name) in releases.chain(presses).copied() {
            if let (true, Some(hid_usage)) = (self.caps.keyboard, key_usage(name)) {
                self.emit(InputEvent::Key { hid_usage, down });
            }
        }
        self.mods.ctrl = now.ctrl;
        self.mods.shift = now.shift;
        self.mods.alt = now.alt;
    }

    fn pointer_moved(&mut self, pos: Pos2, geometry: &Geometry) {
        if !self.caps.pointer || self.touch_active {
            return;
        }
        match geometry.pointer_at(pos) {
            Some(point) => {
                self.hover_inside = true;
                self.pending_move = Some(point);
            }
            None => self.hover_inside = false,
        }
    }

    fn pointer_button(&mut self, pos: Pos2, button: EguiButton, pressed: bool, g: &Geometry) {
        if !self.caps.pointer || (self.touch_active && button == EguiButton::Primary) {
            return;
        }
        let mapped = button_of(button);
        if pressed {
            // A click beside the picture (the letterbox) is not for the host.
            if let Some(point) = g.pointer_at(pos) {
                self.pending_move = Some(point);
                self.held_buttons.push(mapped);
                self.emit(InputEvent::Button {
                    button: mapped,
                    down: true,
                });
            }
        } else if self.held_buttons.contains(&mapped) {
            self.pending_move = Some(g.clamped(pos));
            self.held_buttons.retain(|held| *held != mapped);
            self.emit(InputEvent::Button {
                button: mapped,
                down: false,
            });
        }
    }

    /// Wheel notches (up and right positive); a notch's rest stays.
    fn wheel(&mut self, unit: MouseWheelUnit, dx: f32, dy: f32) {
        if !self.caps.pointer || !self.hover_inside {
            return;
        }
        let scale = match unit {
            MouseWheelUnit::Line => NOTCH,
            MouseWheelUnit::Point => NOTCH / POINTS_PER_NOTCH,
            MouseWheelUnit::Page => NOTCH * LINES_PER_PAGE,
        };
        // egui counts the way the content moves (down and right positive).
        let total_x = self.wheel_rest.0 - dx * scale;
        let total_y = self.wheel_rest.1 + dy * scale;
        let (whole_x, whole_y) = (total_x.trunc(), total_y.trunc());
        self.wheel_rest = (total_x - whole_x, total_y - whole_y);
        let clamp = |value: f32| value.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
        let (x, y) = (clamp(whole_x), clamp(whole_y));
        if x != 0 || y != 0 {
            self.emit(InputEvent::Wheel { dx: x, dy: y });
        }
    }

    fn key(&mut self, key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) {
        // A key the host got is let go there whatever the modifiers are now
        // (Codex 5/14: Ctrl+S, then Shift, then S up read as a shortcut).
        let forwarded =
            key_usage_of(key).filter(|usage| !pressed && self.keys_down.contains(usage));
        if let Some(hid_usage) = forwarded {
            self.keys_down.retain(|down| *down != hid_usage);
            self.emit(InputEvent::Key {
                hid_usage,
                down: false,
            });
            return;
        }
        if local_key(key, modifiers).is_some() {
            if !pressed {
                self.swallowed.retain(|held| *held != key);
            }
            if pressed && !self.swallowed.contains(&key) {
                self.swallowed.push(key);
            }
            return;
        }
        if !pressed {
            if let Some(index) = self.swallowed.iter().position(|held| *held == key) {
                self.swallowed.remove(index);
                return;
            }
            if let Some(index) = self.text_keys.iter().position(|held| *held == key) {
                self.text_keys.remove(index);
                return;
            }
        } else if repeat || self.swallowed.contains(&key) || self.text_keys.contains(&key) {
            // The host repeats a held key itself.
            return;
        }
        if pressed && self.caps.text && is_text_key(key) && !modifiers.ctrl && !modifiers.alt {
            // The character arrives as `Text`; the key stays with the viewer.
            self.text_keys.push(key);
            return;
        }
        let Some(hid_usage) = key_usage_of(key) else {
            if pressed {
                self.unmapped_keys += 1;
            }
            return;
        };
        if !self.caps.keyboard {
            return;
        }
        if pressed {
            if !self.keys_down.contains(&hid_usage) {
                self.keys_down.push(hid_usage);
            }
        } else if let Some(index) = self.keys_down.iter().position(|down| *down == hid_usage) {
            self.keys_down.remove(index);
        } else {
            // A release without a press the host got is never sent.
            return;
        }
        self.emit(InputEvent::Key {
            hid_usage,
            down: pressed,
        });
    }

    fn touch(&mut self, id: u64, phase: EguiPhase, pos: Pos2, g: &Geometry) {
        if !self.caps.touch {
            return;
        }
        let phase = phase_of(phase);
        let inside = g.pointer_at(pos);
        // A touch may end beside the picture; only its start must be on it.
        let point = match (phase, inside) {
            (TouchPhase::Start, None) => return,
            (_, Some(point)) => point,
            (_, None) => g.clamped(pos),
        };
        self.touch_active = !matches!(phase, TouchPhase::End | TouchPhase::Cancel);
        self.emit(InputEvent::Touch {
            id: (id & 0xFF) as u8,
            phase,
            x: point.0,
            y: point.1,
        });
    }
}

#[path = "screen_input_paste.rs"]
mod paste;
#[path = "screen_input_text.rs"]
mod text;

#[cfg(test)]
#[path = "screen_input_batch_tests.rs"]
mod batch_tests;
#[cfg(test)]
#[path = "screen_input_map_tests.rs"]
mod tests;
