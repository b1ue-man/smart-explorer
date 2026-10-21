//! The surface on an X11 desktop (U34-3, DSK-M6): the work area of a monitor
//! from `_NET_WORKAREA` (the monitors themselves: `x11_monitors`), and the
//! states that put the window under the others on every workspace without a
//! task-bar or pager entry.
//!
//! The window type DESKTOP is set before the window is shown (`ViewportBuilder`
//! in `desktop_surface_app`). The states follow here: on a window that is not
//! shown yet as the initial value of the properties (the window manager reads
//! them when it maps the window), on a shown one as `_NET_WM_STATE` and
//! `_NET_WM_DESKTOP` client messages to the root (EWMH 1.4: only the window
//! manager changes the properties of a shown window). A watch thread repeats
//! the messages when the window is mapped and tells the view when the work
//! area or the monitors change.
use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, EventMask,
    MapState, PropMode, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::probe_timeout::timed;
use super::session_facts::{self, SystemProbe};
use super::x11_facts::{intern, values32};
use super::x11_monitors::list_monitors;
use crate::app::desktop_surface_host::{NativeWindow, SurfaceSignal, SurfaceSignals};
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::mechanism::{self, Mechanism};
use crate::desktop::os::store::DesktopDir;

/// `_NET_WM_DESKTOP` value "on all workspaces".
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;
/// Source indication of a client message: a normal application.
const SOURCE_APPLICATION: u32 = 1;
/// `_NET_WM_STATE_ADD` (EWMH: 0 removes, 1 adds, 2 toggles).
const STATE_ADD: u32 = 1;

/// The states of the surface window per mechanism (C2.1 L1, L2).
pub(crate) fn state_names(mechanism: &Mechanism) -> &'static [&'static str] {
    match mechanism {
        Mechanism::X11DesktopType => &[
            "_NET_WM_STATE_SKIP_TASKBAR",
            "_NET_WM_STATE_SKIP_PAGER",
            "_NET_WM_STATE_STICKY",
        ],
        Mechanism::X11Below => &[
            "_NET_WM_STATE_BELOW",
            "_NET_WM_STATE_SKIP_TASKBAR",
            "_NET_WM_STATE_SKIP_PAGER",
            "_NET_WM_STATE_STICKY",
        ],
        _ => &[],
    }
}

/// The work area of workspace `current` in `_NET_WORKAREA` (`x y width
/// height` per workspace); an index beyond the list takes the first entry.
pub(crate) fn current_work_area(values: &[u32], current: Option<u32>) -> Option<[i32; 4]> {
    let count = values.len() / 4;
    if count == 0 {
        return None;
    }
    let index = current
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < count)
        .unwrap_or(0);
    let entry = &values[index * 4..index * 4 + 4];
    // The values are CARDINALs holding signed pixels.
    Some([
        entry[0] as i32,
        entry[1] as i32,
        entry[2] as i32,
        entry[3] as i32,
    ])
}

/// `work` (`[x, y, width, height]`) cut with the monitor's `[left, top,
/// right, bottom]`, as `[left, top, right, bottom]`; `None` when they do not
/// overlap.
pub(crate) fn intersect_work_area(work: [i32; 4], monitor: [i32; 4]) -> Option<[i32; 4]> {
    let left = work[0].max(monitor[0]);
    let top = work[1].max(monitor[1]);
    let right = work[0].saturating_add(work[2]).min(monitor[2]);
    let bottom = work[1].saturating_add(work[3]).min(monitor[3]);
    (right > left && bottom > top).then_some([left, top, right, bottom])
}

fn read_work_area(key: String) -> Option<[i32; 4]> {
    let (conn, screen) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let rect = list_monitors(&conn, root, screen)
        .into_iter()
        .find(|monitor| monitor.key == key)?
        .rect?;
    let workarea = values32(
        &conn,
        root,
        intern(&conn, "_NET_WORKAREA")?,
        AtomEnum::CARDINAL,
    );
    let current = intern(&conn, "_NET_CURRENT_DESKTOP")
        .map(|atom| values32(&conn, root, atom, AtomEnum::CARDINAL))
        .and_then(|values| values.first().copied());
    intersect_work_area(current_work_area(&workarea, current)?, rect)
}

/// The monitor without panels and docks (`_NET_WORKAREA` of the current
/// workspace cut with the monitor); `None` while the window manager names no
/// work area, which shows the whole monitor.
pub(crate) fn work_area(monitor: &str) -> Option<[i32; 4]> {
    let key = monitor.to_string();
    timed("x11-work-area", move || read_work_area(key)).flatten()
}

fn window_of(window: NativeWindow) -> Option<Window> {
    match window {
        NativeWindow::Xlib { window } => u32::try_from(window).ok(),
        NativeWindow::Xcb { window } => Some(window),
        _ => None,
    }
}

/// Puts the window under the others (see the module text) and starts the
/// watch thread. Other windows than X11 ones are left alone.
pub(crate) fn attach_surface(
    window: NativeWindow,
    monitor: &str,
    signals: Arc<SurfaceSignals>,
) -> Result<(), String> {
    let Some(window) = window_of(window) else {
        return Ok(());
    };
    // The file clipboard of an X11 surface (one provider per process).
    super::x11_clipboard::install_once();
    let monitor = monitor.to_string();
    std::thread::Builder::new()
        .name("x11-surface-watch".into())
        .spawn(move || {
            if let Err(error) = watch(window, &signals) {
                eprintln!("se Desktop: X11-Fenster {monitor}: {error}");
            }
        })
        .map(drop)
        .map_err(|error| error.to_string())
}

fn mechanism_now() -> Mechanism {
    let icons = DesktopDir::production()
        .read_config()
        .ok()
        .flatten()
        .map_or(ForeignDesktopIcons::Keep, |config| {
            config.foreign_desktop_icons
        });
    mechanism::decide(&session_facts::probe(&SystemProbe), icons)
}

struct Atoms {
    state: Atom,
    desktop: Atom,
    wanted: Vec<Atom>,
    workarea: Atom,
    current: Atom,
}

fn send_message(
    conn: &RustConnection,
    root: Window,
    window: Window,
    kind: Atom,
    data: [u32; 5],
) -> Result<(), String> {
    let event = ClientMessageEvent::new(32, window, kind, data);
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    conn.send_event(false, root, mask, event)
        .map_err(|error| error.to_string())?;
    conn.flush().map_err(|error| error.to_string())
}

/// The states and the workspace as client messages (shown window).
fn announce(conn: &RustConnection, root: Window, window: Window, atoms: &Atoms) {
    for pair in atoms.wanted.chunks(2) {
        let second = pair.get(1).copied().unwrap_or(0);
        let data = [STATE_ADD, pair[0], second, SOURCE_APPLICATION, 0];
        let _ = send_message(conn, root, window, atoms.state, data);
    }
    let data = [ALL_DESKTOPS, SOURCE_APPLICATION, 0, 0, 0];
    let _ = send_message(conn, root, window, atoms.desktop, data);
}

/// The initial values of a window that is not shown yet.
fn set_initial(conn: &RustConnection, window: Window, atoms: &Atoms) {
    let mut states = values32(conn, window, atoms.state, AtomEnum::ATOM);
    for atom in &atoms.wanted {
        if !states.contains(atom) {
            states.push(*atom);
        }
    }
    let _ = conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.state,
        AtomEnum::ATOM,
        &states,
    );
    let _ = conn.change_property32(
        PropMode::REPLACE,
        window,
        atoms.desktop,
        AtomEnum::CARDINAL,
        &[ALL_DESKTOPS],
    );
    let _ = conn.flush();
}

fn watch(window: Window, signals: &SurfaceSignals) -> Result<(), String> {
    let failed = |error: &dyn std::fmt::Display| error.to_string();
    let mechanism = mechanism_now();
    let names = state_names(&mechanism);
    if names.is_empty() {
        // `none:*` never starts a surface; a window that is here anyway
        // stays as the window manager made it.
        return Ok(());
    }
    let (conn, screen) = RustConnection::connect(None).map_err(|error| failed(&error))?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or("kein Bildschirm")?;
    let need = |name: &str| intern(&conn, name).ok_or_else(|| format!("Atom {name}"));
    let atoms = Atoms {
        state: need("_NET_WM_STATE")?,
        desktop: need("_NET_WM_DESKTOP")?,
        wanted: names
            .iter()
            .map(|name| need(name))
            .collect::<Result<_, _>>()?,
        workarea: need("_NET_WORKAREA")?,
        current: need("_NET_CURRENT_DESKTOP")?,
    };
    // The MapNotify of the window is selected before its map state is read:
    // a window mapped between the two would otherwise bring no MapNotify,
    // get no messages and keep only what the window manager read when it
    // mapped it (`_NET_WM_STATE_BELOW` was missing under xfwm4 in suite run
    // 7 while the other states were there).
    let own = ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY);
    let _ = conn.change_window_attributes(window, &own);
    let shown = conn
        .get_window_attributes(window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.map_state == MapState::VIEWABLE);
    let desktop = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    let _ = conn.change_window_attributes(root, &desktop);
    let _ = conn.randr_select_input(
        root,
        randr::NotifyMask::SCREEN_CHANGE
            | randr::NotifyMask::CRTC_CHANGE
            | randr::NotifyMask::OUTPUT_CHANGE,
    );
    if shown {
        announce(&conn, root, window, &atoms);
    } else {
        set_initial(&conn, window, &atoms);
    }
    conn.flush().map_err(|error| failed(&error))?;
    loop {
        match conn.wait_for_event().map_err(|error| failed(&error))? {
            Event::MapNotify(event) if event.window == window => {
                announce(&conn, root, window, &atoms);
            }
            Event::PropertyNotify(event)
                if event.atom == atoms.workarea || event.atom == atoms.current =>
            {
                signals.raise(SurfaceSignal::Display);
            }
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_) => {
                // The shell reads the monitors every second and starts the
                // surfaces anew; the view reads its work area again.
                signals.raise(SurfaceSignal::Display);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "x11_task_tests.rs"]
mod tests;
