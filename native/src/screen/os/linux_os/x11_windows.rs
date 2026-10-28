//! The windows of an X11 session (SCR-M5, U19-6/7): the list from the
//! window manager's EWMH properties (`_NET_CLIENT_LIST`, `_NET_WM_NAME`,
//! `_NET_WM_PID`, `WM_CLASS`, `_NET_WM_STATE`, `_NET_ACTIVE_WINDOW`), and
//! without an EWMH window manager the viewable children of the root window
//! with a title (`QueryTree`). EWMH atoms are assumption A15. Window ids are
//! `window:<XID in decimal>`; windows smaller than 2 × 2 pixels stay out.
use std::io;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, MapState, Window};
use x11rb::rust_connection::RustConnection;

use super::x11_monitors;
use crate::screen::core::window_list::{ListSource, MonitorInfo, WindowEntry, WindowList};

/// Smallest side of a window that is listed.
const MIN_SIDE: u16 = 2;

/// `window:<id>` of the X11 window `xid`.
pub(crate) fn window_id(xid: u32) -> String {
    xid.to_string()
}

/// The X11 window of a source id (`window:<id>`), if it is a decimal XID.
pub(crate) fn parse_window_id(id: &str) -> Option<u32> {
    id.parse().ok()
}

/// `WM_CLASS` is two NUL-terminated strings, the instance and the class;
/// the app is the class, or the instance when the class is empty.
pub(crate) fn app_of_wm_class(value: &[u8]) -> String {
    let mut parts = value.split(|byte| *byte == 0);
    let instance = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
    let class = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
    if class.is_empty() {
        instance
    } else {
        class
    }
}

/// The id of the monitor containing the point (`x`, `y`).
pub(crate) fn monitor_at(monitors: &[MonitorInfo], x: i32, y: i32) -> Option<String> {
    monitors
        .iter()
        .find(|monitor| {
            x >= monitor.x
                && y >= monitor.y
                && i64::from(x) < i64::from(monitor.x) + i64::from(monitor.width)
                && i64::from(y) < i64::from(monitor.y) + i64::from(monitor.height)
        })
        .map(|monitor| monitor.id.clone())
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("x11: {error}"))
}

/// The atoms this module reads.
pub(crate) struct Atoms {
    pub client_list: Atom,
    pub supporting_wm_check: Atom,
    pub active_window: Atom,
    pub wm_name: Atom,
    pub wm_pid: Atom,
    pub wm_state: Atom,
    pub wm_state_hidden: Atom,
    pub utf8_string: Atom,
    pub wm_class: Atom,
    pub wm_cm_selection: Atom,
}

impl Atoms {
    /// `screen_number` selects `_NET_WM_CM_S<n>`.
    pub(crate) fn intern<C: Connection>(conn: &C, screen_number: usize) -> io::Result<Self> {
        let names: [String; 10] = [
            "_NET_CLIENT_LIST".into(),
            "_NET_SUPPORTING_WM_CHECK".into(),
            "_NET_ACTIVE_WINDOW".into(),
            "_NET_WM_NAME".into(),
            "_NET_WM_PID".into(),
            "_NET_WM_STATE".into(),
            "_NET_WM_STATE_HIDDEN".into(),
            "UTF8_STRING".into(),
            "WM_CLASS".into(),
            format!("_NET_WM_CM_S{screen_number}"),
        ];
        let mut cookies = Vec::with_capacity(names.len());
        for name in &names {
            cookies.push(conn.intern_atom(false, name.as_bytes()).map_err(failed)?);
        }
        let mut atoms = Vec::with_capacity(names.len());
        for cookie in cookies {
            atoms.push(cookie.reply().map_err(failed)?.atom);
        }
        Ok(Self {
            client_list: atoms[0],
            supporting_wm_check: atoms[1],
            active_window: atoms[2],
            wm_name: atoms[3],
            wm_pid: atoms[4],
            wm_state: atoms[5],
            wm_state_hidden: atoms[6],
            utf8_string: atoms[7],
            wm_class: atoms[8],
            wm_cm_selection: atoms[9],
        })
    }
}

/// One property as bytes (empty when absent).
fn property<C: Connection>(
    conn: &C,
    window: Window,
    property: Atom,
    kind: impl Into<Atom>,
) -> io::Result<Vec<u8>> {
    let reply = conn
        .get_property(false, window, property, kind, 0, u32::MAX)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok(reply.value)
}

/// One property of 32-bit values.
fn words<C: Connection>(
    conn: &C,
    window: Window,
    name: Atom,
    kind: impl Into<Atom>,
) -> io::Result<Vec<u32>> {
    let reply = conn
        .get_property(false, window, name, kind, 0, u32::MAX)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok(reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default())
}

/// The title of `window`: `_NET_WM_NAME`, else `WM_NAME`.
pub(crate) fn title<C: Connection>(conn: &C, atoms: &Atoms, window: Window) -> io::Result<String> {
    let utf8 = property(conn, window, atoms.wm_name, atoms.utf8_string)?;
    if !utf8.is_empty() {
        return Ok(String::from_utf8_lossy(&utf8).into_owned());
    }
    let legacy = property(conn, window, AtomEnum::WM_NAME.into(), AtomEnum::ANY)?;
    Ok(String::from_utf8_lossy(&legacy).into_owned())
}

/// The window's position on the root and its size.
pub(crate) fn root_geometry<C: Connection>(
    conn: &C,
    root: Window,
    window: Window,
) -> io::Result<(i32, i32, u16, u16)> {
    let geometry = conn
        .get_geometry(window)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let origin = conn
        .translate_coordinates(window, root, 0, 0)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok((
        i32::from(origin.dst_x),
        i32::from(origin.dst_y),
        geometry.width,
        geometry.height,
    ))
}

/// Whether `window` is minimized (`_NET_WM_STATE_HIDDEN`).
pub(crate) fn is_hidden<C: Connection>(
    conn: &C,
    atoms: &Atoms,
    window: Window,
) -> io::Result<bool> {
    let states = words(conn, window, atoms.wm_state, AtomEnum::ATOM)?;
    Ok(states.contains(&atoms.wm_state_hidden))
}

/// Whether an EWMH window manager manages the X server of `DISPLAY`: its
/// root names a `_NET_SUPPORTING_WM_CHECK` window, the rule the window list
/// uses too. LightDM greeters run without one and focus every window that
/// is mapped (unity-greeter `focus_upon_map`, lightdm-gtk-greeter
/// `wm_window_filter`), except windows of type NOTIFICATION.
pub(crate) fn window_manager_running() -> io::Result<bool> {
    let (conn, screen_number) = RustConnection::connect(None).map_err(failed)?;
    let root = conn.setup().roots[screen_number].root;
    let check = conn
        .intern_atom(true, b"_NET_SUPPORTING_WM_CHECK")
        .map_err(failed)?
        .reply()
        .map_err(failed)?
        .atom;
    // An atom nobody interned yet: no window manager ever announced itself.
    if check == u32::from(AtomEnum::NONE) {
        return Ok(false);
    }
    Ok(!words(&conn, root, check, AtomEnum::WINDOW)?.is_empty())
}

/// The windows of the session of `DISPLAY`.
pub(crate) fn list() -> io::Result<WindowList> {
    let (conn, screen_number) = RustConnection::connect(None).map_err(failed)?;
    let root = conn.setup().roots[screen_number].root;
    list_on(&conn, root, screen_number)
}

pub(crate) fn list_on<C: Connection>(
    conn: &C,
    root: Window,
    screen_number: usize,
) -> io::Result<WindowList> {
    let atoms = Atoms::intern(conn, screen_number)?;
    let monitors = x11_monitors::list(conn, root).unwrap_or_default();
    let manager = !words(conn, root, atoms.supporting_wm_check, AtomEnum::WINDOW)?.is_empty();
    let clients = words(conn, root, atoms.client_list, AtomEnum::WINDOW)?;
    let active = words(conn, root, atoms.active_window, AtomEnum::WINDOW)?
        .first()
        .copied()
        .unwrap_or(0);
    let (windows, list_source) = if manager && !clients.is_empty() {
        let mut windows = Vec::new();
        for window in clients {
            match ewmh_entry(conn, &atoms, root, &monitors, window, active) {
                Ok(Some(entry)) => windows.push(entry),
                // A window that went away between the list and the query.
                Ok(None) | Err(_) => {}
            }
        }
        (windows, ListSource::Ewmh)
    } else {
        (
            query_tree(conn, &atoms, root, &monitors, active)?,
            ListSource::X11QueryTree,
        )
    };
    Ok(WindowList {
        windows,
        apps: Vec::new(),
        list_source,
        limits: Vec::new(),
    })
}

fn ewmh_entry<C: Connection>(
    conn: &C,
    atoms: &Atoms,
    root: Window,
    monitors: &[MonitorInfo],
    window: Window,
    active: Window,
) -> io::Result<Option<WindowEntry>> {
    let (x, y, width, height) = root_geometry(conn, root, window)?;
    if width < MIN_SIDE || height < MIN_SIDE {
        return Ok(None);
    }
    let class = property(conn, window, atoms.wm_class, AtomEnum::STRING)?;
    let pid = words(conn, window, atoms.wm_pid, AtomEnum::CARDINAL)?
        .first()
        .copied();
    Ok(Some(WindowEntry {
        id: window_id(window),
        title: title(conn, atoms, window)?,
        app: app_of_wm_class(&class),
        app_id: None,
        icon: None,
        pid,
        monitor: monitor_at(
            monitors,
            x + i32::from(width) / 2,
            y + i32::from(height) / 2,
        ),
        minimized: is_hidden(conn, atoms, window)?,
        focused: window == active,
    }))
}

/// Without EWMH: the viewable children of the root with a title.
fn query_tree<C: Connection>(
    conn: &C,
    atoms: &Atoms,
    root: Window,
    monitors: &[MonitorInfo],
    active: Window,
) -> io::Result<Vec<WindowEntry>> {
    let tree = conn
        .query_tree(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let mut windows = Vec::new();
    for window in tree.children {
        let attributes = match conn.get_window_attributes(window).map(|c| c.reply()) {
            Ok(Ok(attributes)) => attributes,
            _ => continue,
        };
        if attributes.map_state != MapState::VIEWABLE {
            continue;
        }
        let (Ok(title), Ok((x, y, width, height))) = (
            title(conn, atoms, window),
            root_geometry(conn, root, window),
        ) else {
            continue;
        };
        if title.is_empty() || width < MIN_SIDE || height < MIN_SIDE {
            continue;
        }
        let class = property(conn, window, atoms.wm_class, AtomEnum::STRING).unwrap_or_default();
        windows.push(WindowEntry {
            id: window_id(window),
            title,
            app: app_of_wm_class(&class),
            app_id: None,
            icon: None,
            pid: None,
            monitor: monitor_at(
                monitors,
                x + i32::from(width) / 2,
                y + i32::from(height) / 2,
            ),
            minimized: false,
            focused: window == active,
        });
    }
    Ok(windows)
}
