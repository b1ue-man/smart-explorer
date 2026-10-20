//! What the X11 server tells about the window manager (U34-2): the atoms of
//! `_NET_SUPPORTED`, a desktop window of another program, and a running
//! compositor. Reads only; one connection per call.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::session_facts::X11Facts;

/// `WM_CLASS` of the surfaces (`desktop_surface_app::APP_ID`); a desktop
/// window of this class is our own and never "foreign".
pub(super) const OWN_CLASS: &str = "app.smartexplorer.desktop";
const WINDOW_TYPE: &str = "_NET_WM_WINDOW_TYPE";
const WINDOW_TYPE_DESKTOP: &str = "_NET_WM_WINDOW_TYPE_DESKTOP";
/// Property reads ask for at most this many 32-bit values.
const MAX_VALUES: u32 = 1 << 20;

/// The atom `name`, when the server knows it or lets it be created.
pub(super) fn intern(conn: &RustConnection, name: &str) -> Option<Atom> {
    conn.intern_atom(false, name.as_bytes())
        .ok()?
        .reply()
        .ok()
        .map(|reply| reply.atom)
}

/// The 32-bit values of a property; empty when it is not set.
pub(super) fn values32(
    conn: &RustConnection,
    window: Window,
    property: Atom,
    kind: AtomEnum,
) -> Vec<u32> {
    conn.get_property(false, window, property, kind, 0, MAX_VALUES)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(Iterator::collect))
        .unwrap_or_default()
}

/// The names of `atoms`, asked for in one round.
fn names_of(conn: &RustConnection, atoms: &[Atom]) -> Vec<String> {
    let cookies: Vec<_> = atoms
        .iter()
        .filter_map(|atom| conn.get_atom_name(*atom).ok())
        .collect();
    cookies
        .into_iter()
        .filter_map(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
        .collect()
}

/// Whether `window` belongs to the surfaces (`WM_CLASS` holds [`OWN_CLASS`]).
fn is_own(conn: &RustConnection, window: Window) -> bool {
    conn.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| String::from_utf8_lossy(&reply.value).contains(OWN_CLASS))
}

/// A window of type DESKTOP that is not ours: among the stacked clients and
/// the children of the root (a desktop window may be no managed client).
fn foreign_desktop_window(conn: &RustConnection, root: Window) -> bool {
    let (Some(stacking), Some(kind), Some(desktop)) = (
        intern(conn, "_NET_CLIENT_LIST_STACKING"),
        intern(conn, WINDOW_TYPE),
        intern(conn, WINDOW_TYPE_DESKTOP),
    ) else {
        return false;
    };
    let mut windows = values32(conn, root, stacking, AtomEnum::WINDOW);
    if let Some(tree) = conn
        .query_tree(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    {
        windows.extend(tree.children);
    }
    windows.sort_unstable();
    windows.dedup();
    let cookies: Vec<_> = windows
        .iter()
        .filter_map(|window| {
            let cookie = conn
                .get_property(false, *window, kind, AtomEnum::ATOM, 0, 64)
                .ok()?;
            Some((*window, cookie))
        })
        .collect();
    cookies
        .into_iter()
        .filter_map(|(window, cookie)| {
            let reply = cookie.reply().ok()?;
            let is_desktop = reply.value32()?.any(|atom| atom == desktop);
            is_desktop.then_some(window)
        })
        .any(|window| !is_own(conn, window))
}

/// A compositor owns the selection `_NET_WM_CM_S<screen>`.
fn compositor_running(conn: &RustConnection, screen: usize) -> bool {
    intern(conn, &format!("_NET_WM_CM_S{screen}"))
        .and_then(|atom| conn.get_selection_owner(atom).ok())
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.owner != x11rb::NONE)
}

/// The facts of the server in `DISPLAY`; `None` without a connection.
pub(super) fn read() -> Option<X11Facts> {
    let (conn, screen) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let supported = intern(&conn, "_NET_SUPPORTED")
        .map(|property| values32(&conn, root, property, AtomEnum::ATOM))
        .map(|atoms| names_of(&conn, &atoms))
        .unwrap_or_default();
    Some(X11Facts {
        supported,
        stacking_has_foreign_desktop: foreign_desktop_window(&conn, root),
        compositor: compositor_running(&conn, screen),
    })
}
