//! The input of the desktop viewer (SCR-M7, U25-8): egui events become the
//! `InputEvent`s of the `INPUT` frame. The pointer is mapped to 0–65535
//! across the picture (never outside it), buttons and wheel notches (1/120)
//! follow the HID model, keys become HID usages of `input_model::key_usage`,
//! modifiers go out as the left keys when they change (egui reports no
//! right ones), typing goes out as `Text` (so the host's layout decides the
//! character) and shortcuts with Ctrl or Alt as keys, and an IME commit goes
//! out as `Text`. Every press is tracked (`PressedInputs`), so
//! `release_all` lifts whatever is still down when control ends, the window
//! loses the focus or the session ends. The events of one frame are bundled
//! into batches (`take_batch`); what does not fit goes out with the next
//! frame.
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

pub(crate) use super::screen_viewport::Geometry;

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
pub(crate) struct InputMapper {
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
    wheel_rest: (f32, f32),
    touch_active: bool,
    /// Keys without a usage that were dropped (shown in the statistics).
    pub(crate) unmapped_keys: u64,
    /// The text of a running IME composition (shown, not sent).
    pub(crate) preedit: Option<String>,
}

impl InputMapper {
    pub(crate) fn new(caps: InputCapabilities) -> Self {
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
            wheel_rest: (0.0, 0.0),
            touch_active: false,
            unmapped_keys: 0,
            preedit: None,
        }
    }

    pub(crate) fn has_backlog(&self) -> bool {
        !self.backlog.is_empty()
    }

    /// Everything the host holds down for this viewer is lifted (control
    /// ends, the window lost the focus, the session ends).
    pub(crate) fn release_all(&mut self) {
        self.flush_move();
        let released = self.pressed.release_all();
        self.backlog.extend(released);
        self.held_buttons.clear();
        self.text_keys.clear();
        self.mods = Modifiers::NONE;
        self.suppress_modifiers = true;
        self.touch_active = false;
    }

    /// The next batch for one `INPUT` frame: at most `MAX_INPUT_EVENTS`
    /// events and what the frame takes.
    pub(crate) fn take_batch(&mut self) -> Option<Vec<InputEvent>> {
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
    pub(crate) fn process(&mut self, events: &[Event], frame_mods: Modifiers, geometry: &Geometry) {
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
                pressed,
                repeat,
                modifiers,
                ..
            } => {
                self.sync_modifiers(*modifiers);
                self.key(*key, *pressed, *repeat, *modifiers);
            }
            Event::Text(text) => {
                if self.caps.text && !self.mods.ctrl && !self.mods.alt {
                    self.text(text);
                }
            }
            Event::Ime(ime) => self.ime(ime),
            Event::Copy => self.chord("c"),
            Event::Cut => self.chord("x"),
            Event::Paste(_) => self.chord("v"),
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
        for (down, was, name) in changes {
            if down == was {
                continue;
            }
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

    /// Wheel notches: up and to the right are positive; the rest of a notch
    /// stays for the next event.
    fn wheel(&mut self, unit: MouseWheelUnit, dx: f32, dy: f32) {
        if !self.caps.pointer || !self.hover_inside {
            return;
        }
        let scale = match unit {
            MouseWheelUnit::Line => NOTCH,
            MouseWheelUnit::Point => NOTCH / POINTS_PER_NOTCH,
            MouseWheelUnit::Page => NOTCH * LINES_PER_PAGE,
        };
        // egui counts the way the content moves: down is positive, but the
        // wheel turned up scrolls up; right is positive, scrolling left.
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
        if self.caps.keyboard {
            self.emit(InputEvent::Key {
                hid_usage,
                down: pressed,
            });
        }
    }

    /// Ctrl+C, X and V arrive as their own events without a key event.
    fn chord(&mut self, name: &str) {
        let Some(hid_usage) = key_usage(name).filter(|_| self.caps.keyboard) else {
            return;
        };
        self.sync_modifiers(Modifiers {
            ctrl: true,
            ..self.mods
        });
        self.emit(InputEvent::Key {
            hid_usage,
            down: true,
        });
        self.emit(InputEvent::Key {
            hid_usage,
            down: false,
        });
    }

    /// `Text` events of at most `MAX_TEXT_BYTES`, without NUL.
    fn text(&mut self, text: &str) {
        let mut rest: String = text.chars().filter(|c| *c != '\0').collect();
        while !rest.is_empty() {
            let mut end = rest.len().min(MAX_TEXT_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let tail = rest.split_off(end);
            let head = std::mem::replace(&mut rest, tail);
            self.emit(InputEvent::Text { text: head });
        }
    }

    fn ime(&mut self, ime: &ImeEvent) {
        match ime {
            ImeEvent::Commit(text) => {
                self.preedit = None;
                if self.caps.text {
                    self.text(text);
                }
            }
            ImeEvent::Preedit(text) => {
                self.preedit = (!text.is_empty()).then(|| text.clone());
            }
            ImeEvent::Enabled => {}
            ImeEvent::Disabled => self.preedit = None,
        }
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

#[cfg(test)]
#[path = "screen_input_batch_tests.rs"]
mod batch_tests;
#[cfg(test)]
#[path = "screen_input_map_tests.rs"]
mod tests;
