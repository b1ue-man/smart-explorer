//! The seats of the layer process (DSK-M7, U37-10, U37-11): pointer (with
//! the cursor shape or the cursor theme), touch and keyboard devices as the
//! seats offer them, the modifiers all keyboards share and the mapping of
//! egui's cursor to the toolkit's. The events themselves are in
//! `pointer.rs` and `keyboard.rs`.
use eframe::egui::{self, Modifiers};
use smithay_client_toolkit::compositor::SurfaceData;
use smithay_client_toolkit::seat::pointer::{CursorIcon, ThemeSpec, ThemedPointer};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_touch::WlTouch;
use wayland_client::{Connection, Proxy, QueueHandle};

use super::keyboard::KeyboardEntry;
use super::state::LayerState;
use super::view::OutputView;
use super::xkb::Xkb;

/// The pointer of a seat: with cursor changes when the compositor has
/// `wp_cursor_shape_v1` or `wl_shm` for the cursor theme, else plain.
pub(super) enum PointerDevice {
    Themed(ThemedPointer),
    Plain(WlPointer),
}

/// The devices of one seat.
pub(super) struct SeatDevices {
    pub(super) seat: WlSeat,
    pub(super) pointer: Option<PointerDevice>,
    pub(super) touch: Option<WlTouch>,
}

#[derive(Default)]
pub(super) struct Input {
    pub(super) seats: Vec<SeatDevices>,
    pub(super) keyboards: Vec<KeyboardEntry>,
    /// Alt, Ctrl and Shift as the keyboards last reported them.
    pub(super) modifiers: Modifiers,
    /// The touch point that also acts as the pointer.
    pub(super) first_touch: Option<i32>,
    /// Active touch points and the surfaces they began on.
    pub(super) touches: Vec<(i32, WlSurface)>,
    /// The last position of each active touch point (`wl_touch.up` has none).
    pub(super) touch_positions: Vec<(i32, egui::Pos2)>,
    /// The cursor egui asked for last.
    pub(super) cursor: Option<egui::CursorIcon>,
}

impl Input {
    pub(super) fn seat_mut(&mut self, seat: &WlSeat) -> &mut SeatDevices {
        if let Some(index) = self.seats.iter().position(|devices| &devices.seat == seat) {
            return &mut self.seats[index];
        }
        self.seats.push(SeatDevices {
            seat: seat.clone(),
            pointer: None,
            touch: None,
        });
        let last = self.seats.len() - 1;
        &mut self.seats[last]
    }

    /// Sets the cursor on every pointer that can; a pointer that has not
    /// entered a surface yet keeps the compositor's cursor.
    pub(super) fn apply_cursor(&mut self, connection: &Connection, icon: egui::CursorIcon) {
        self.cursor = Some(icon);
        for devices in &self.seats {
            if let Some(PointerDevice::Themed(pointer)) = &devices.pointer {
                let _ = match cursor_for(icon) {
                    Some(icon) => pointer.set_cursor(connection, icon),
                    None => pointer.hide_cursor(),
                };
            }
        }
    }

    /// The next key repeat of any keyboard.
    pub(super) fn next_repeat(&self) -> Option<std::time::Instant> {
        self.keyboards
            .iter()
            .filter_map(|entry| entry.held_next())
            .min()
    }
}

/// egui's cursor as the toolkit's (U37-10): default, pointing hand (pointer),
/// text, grab, grabbing, the two plain resize cursors and "not allowed";
/// anything else is the default cursor; `None` hides the cursor.
pub(super) fn cursor_for(icon: egui::CursorIcon) -> Option<CursorIcon> {
    Some(match icon {
        egui::CursorIcon::None => return None,
        egui::CursorIcon::PointingHand => CursorIcon::Pointer,
        egui::CursorIcon::Text => CursorIcon::Text,
        egui::CursorIcon::Grab => CursorIcon::Grab,
        egui::CursorIcon::Grabbing => CursorIcon::Grabbing,
        egui::CursorIcon::ResizeHorizontal => CursorIcon::EwResize,
        egui::CursorIcon::ResizeVertical => CursorIcon::NsResize,
        egui::CursorIcon::NotAllowed => CursorIcon::NotAllowed,
        _ => CursorIcon::Default,
    })
}

impl LayerState {
    /// The view whose surface is `surface`.
    pub(super) fn view_for(&mut self, surface: &WlSurface) -> Option<&mut OutputView> {
        self.views
            .iter_mut()
            .find(|view| &view.window.surface == surface)
    }

    /// Queues events on the view of `surface`.
    pub(super) fn push_events(&mut self, surface: &WlSurface, events: Vec<egui::Event>) {
        if events.is_empty() {
            return;
        }
        if let Some(view) = self.view_for(surface) {
            view.push(events);
        }
    }

    /// The data device of a seat (once) and its devices as it gets them.
    pub(super) fn attach_seat(&mut self, seat: &WlSeat) {
        if let Some(clipboard) = &self.clipboard {
            clipboard.add_seat(seat);
        }
    }

    fn add_pointer(&mut self, seat: &WlSeat) {
        let qh = self.qh.clone();
        let device = match &self.shm {
            Some(shm) => {
                let cursor_surface = self.compositor.create_surface(&qh);
                self.seat_state
                    .get_pointer_with_theme::<LayerState, SurfaceData>(
                        &qh,
                        seat,
                        shm.wl_shm(),
                        cursor_surface,
                        ThemeSpec::System,
                    )
                    .map(PointerDevice::Themed)
            }
            None => self
                .seat_state
                .get_pointer(&qh, seat)
                .map(PointerDevice::Plain),
        };
        if let Ok(device) = device {
            self.input.seat_mut(seat).pointer = Some(device);
        }
    }

    fn add_keyboard(&mut self, seat: &WlSeat) {
        if !self.params.keyboard_on_demand {
            return;
        }
        let Some(xkb) = Xkb::new() else {
            eprintln!("se Desktop: Layer-Backend: libxkbcommon.so.0 fehlt, Tastatur aus");
            return;
        };
        let keyboard = seat.get_keyboard(&self.qh, ());
        self.input
            .keyboards
            .push(KeyboardEntry::new(keyboard, seat.clone(), xkb));
    }
}

impl SeatHandler for LayerState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.attach_seat(&seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => self.add_pointer(&seat),
            Capability::Touch => {
                if let Ok(touch) = self.seat_state.get_touch(qh, &seat) {
                    self.input.seat_mut(&seat).touch = Some(touch);
                }
            }
            Capability::Keyboard => self.add_keyboard(&seat),
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                // A themed pointer releases itself when dropped.
                if let Some(PointerDevice::Plain(pointer)) =
                    self.input.seat_mut(&seat).pointer.take()
                {
                    if pointer.version() >= 3 {
                        pointer.release();
                    }
                }
            }
            Capability::Touch => {
                if let Some(touch) = self.input.seat_mut(&seat).touch.take() {
                    if touch.version() >= 3 {
                        touch.release();
                    }
                }
            }
            Capability::Keyboard => {
                self.input.keyboards.retain(|entry| {
                    let keep = entry.seat != seat;
                    if !keep {
                        entry.release();
                    }
                    keep
                });
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.input.seats.retain(|devices| devices.seat != seat);
        self.input.keyboards.retain(|entry| entry.seat != seat);
        if let Some(clipboard) = &self.clipboard {
            clipboard.remove_seat(&seat);
        }
    }
}
