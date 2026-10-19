//! Pointer and touch of the layer backend (DSK-M7, U37-10): `wl_pointer` and
//! `wl_touch` events become egui's pointer, wheel and touch events. The
//! coordinates are the surface's own, in points. Buttons: left, right,
//! middle, side (back) and extra (forward). A wheel notch is one line, a
//! smooth scroll is in points. The first finger of a touch also acts as the
//! pointer, as in egui-winit.
use eframe::egui::{self, Event, MouseWheelUnit, PointerButton, Pos2, TouchId, TouchPhase};
use smithay_client_toolkit::seat::pointer::{
    PointerData, PointerEvent, PointerEventKind, PointerHandler, BTN_EXTRA, BTN_LEFT, BTN_MIDDLE,
    BTN_RIGHT, BTN_SIDE,
};
use smithay_client_toolkit::seat::touch::TouchHandler;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_touch::WlTouch;
use wayland_client::{Connection, Proxy, QueueHandle};

use super::state::LayerState;

/// The egui button of a Linux input button code.
pub(super) fn button_of(code: u32) -> Option<PointerButton> {
    match code {
        BTN_LEFT => Some(PointerButton::Primary),
        BTN_RIGHT => Some(PointerButton::Secondary),
        BTN_MIDDLE => Some(PointerButton::Middle),
        BTN_SIDE => Some(PointerButton::Extra1),
        BTN_EXTRA => Some(PointerButton::Extra2),
        _ => None,
    }
}

/// The wheel event of an axis frame: notches (`discrete`) count as lines,
/// else the smooth value is in points. Wayland's positive axis points down
/// and right, egui's delta moves the content, so both are negated.
pub(super) fn wheel_delta(
    horizontal: (f64, i32),
    vertical: (f64, i32),
) -> Option<(MouseWheelUnit, egui::Vec2)> {
    let notched = horizontal.1 != 0 || vertical.1 != 0;
    let (unit, x, y) = if notched {
        (
            MouseWheelUnit::Line,
            -(horizontal.1 as f32),
            -(vertical.1 as f32),
        )
    } else {
        (
            MouseWheelUnit::Point,
            -(horizontal.0 as f32),
            -(vertical.0 as f32),
        )
    };
    (x != 0.0 || y != 0.0).then(|| (unit, egui::vec2(x, y)))
}

impl LayerState {
    fn note_input(&self, seat: &WlSeat, serial: u32) {
        if let Some(clipboard) = &self.clipboard {
            clipboard.note_serial(seat, serial);
        }
    }

    fn pointer_event(
        &mut self,
        seat: Option<&WlSeat>,
        connection: &Connection,
        event: &PointerEvent,
    ) {
        let position = Pos2::new(event.position.0 as f32, event.position.1 as f32);
        let modifiers = self.input.modifiers;
        let mut events = Vec::new();
        match &event.kind {
            PointerEventKind::Enter { serial } => {
                if let Some(seat) = seat {
                    self.note_input(seat, *serial);
                }
                // The compositor forgets the cursor on every enter.
                if let Some(icon) = self.input.cursor {
                    self.input.apply_cursor(connection, icon);
                }
                events.push(Event::PointerMoved(position));
            }
            PointerEventKind::Leave { .. } => events.push(Event::PointerGone),
            PointerEventKind::Motion { .. } => events.push(Event::PointerMoved(position)),
            PointerEventKind::Press { button, serial, .. }
            | PointerEventKind::Release { button, serial, .. } => {
                if let Some(seat) = seat {
                    self.note_input(seat, *serial);
                }
                let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                if let Some(button) = button_of(*button) {
                    events.push(Event::PointerButton {
                        pos: position,
                        button,
                        pressed,
                        modifiers,
                    });
                }
            }
            PointerEventKind::Axis {
                horizontal,
                vertical,
                ..
            } => {
                let wheel = wheel_delta(
                    (horizontal.absolute, horizontal.discrete),
                    (vertical.absolute, vertical.discrete),
                );
                if let Some((unit, delta)) = wheel {
                    events.push(Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                    });
                }
            }
        }
        match &event.kind {
            PointerEventKind::Enter { .. } => self.set_pointer_inside(&event.surface, true),
            PointerEventKind::Leave { .. } => self.set_pointer_inside(&event.surface, false),
            _ => {}
        }
        self.push_events(&event.surface, events);
    }

    fn set_pointer_inside(&mut self, surface: &WlSurface, inside: bool) {
        if let Some(view) = self.view_for(surface) {
            view.frame.pointer_inside = inside;
        }
    }

    fn touch_events(&mut self, surface: &WlSurface, id: i32, phase: TouchPhase, pos: Pos2) {
        let modifiers = self.input.modifiers;
        let mut events = vec![Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: TouchId::from(id),
            phase,
            pos,
            force: None,
        }];
        let first = self.input.first_touch == Some(id)
            || (phase == TouchPhase::Start && self.input.first_touch.is_none());
        if first {
            match phase {
                TouchPhase::Start => {
                    self.input.first_touch = Some(id);
                    events.push(Event::PointerMoved(pos));
                    events.push(Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    });
                }
                TouchPhase::Move => events.push(Event::PointerMoved(pos)),
                TouchPhase::End | TouchPhase::Cancel => {
                    self.input.first_touch = None;
                    if phase == TouchPhase::End {
                        events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers,
                        });
                    }
                    events.push(Event::PointerGone);
                }
            }
        }
        self.push_events(surface, events);
    }
}

impl PointerHandler for LayerState {
    fn pointer_frame(
        &mut self,
        connection: &Connection,
        _: &QueueHandle<Self>,
        pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        let seat = pointer
            .data::<PointerData>()
            .map(|data| data.seat().clone());
        for event in events {
            self.pointer_event(seat.as_ref(), connection, event);
        }
    }
}

impl TouchHandler for LayerState {
    fn down(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        touch: &WlTouch,
        serial: u32,
        _: u32,
        surface: WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        use smithay_client_toolkit::seat::touch::TouchData;
        if let Some(seat) = touch.data::<TouchData>().map(|data| data.seat().clone()) {
            self.note_input(&seat, serial);
        }
        self.input.touches.retain(|(known, _)| *known != id);
        self.input.touches.push((id, surface.clone()));
        let pos = Pos2::new(position.0 as f32, position.1 as f32);
        self.input.touch_positions.retain(|(known, _)| *known != id);
        self.input.touch_positions.push((id, pos));
        self.touch_events(&surface, id, TouchPhase::Start, pos);
    }

    fn up(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlTouch, _: u32, _: u32, id: i32) {
        let Some(index) = self
            .input
            .touches
            .iter()
            .position(|(known, _)| *known == id)
        else {
            return;
        };
        let (_, surface) = self.input.touches.remove(index);
        // The position of the end is the last one egui saw.
        let pos = self.last_touch_position(id);
        self.touch_events(&surface, id, TouchPhase::End, pos);
    }

    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlTouch,
        _: u32,
        id: i32,
        position: (f64, f64),
    ) {
        let Some(surface) = self
            .input
            .touches
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, surface)| surface.clone())
        else {
            return;
        };
        let pos = Pos2::new(position.0 as f32, position.1 as f32);
        self.input.touch_positions.retain(|(known, _)| *known != id);
        self.input.touch_positions.push((id, pos));
        self.touch_events(&surface, id, TouchPhase::Move, pos);
    }

    fn shape(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlTouch,
        _: i32,
        _: f64,
        _: f64,
    ) {
    }

    fn orientation(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlTouch, _: i32, _: f64) {}

    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlTouch) {
        let touches = std::mem::take(&mut self.input.touches);
        for (id, surface) in touches {
            let pos = self.last_touch_position(id);
            self.touch_events(&surface, id, TouchPhase::Cancel, pos);
        }
        self.input.first_touch = None;
        self.input.touch_positions.clear();
    }
}

impl LayerState {
    /// Where a touch point was last seen.
    fn last_touch_position(&mut self, id: i32) -> Pos2 {
        let position = self
            .input
            .touch_positions
            .iter()
            .find(|(known, _)| *known == id)
            .map_or(Pos2::ZERO, |(_, pos)| *pos);
        self.input.touch_positions.retain(|(known, _)| *known != id);
        position
    }
}
