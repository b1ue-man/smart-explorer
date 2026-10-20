//! The monitors of an X11 session (U34-3): RandR 1.5 `GetMonitors`, else the
//! CRTCs of the current screen resources, else the root window as the one
//! monitor `screen`. The key is the name of the monitor's first output
//! (`randr::get_output_info(output).name`, the monitor's own name for a
//! virtual monitor without outputs), cleaned and unique (`monitor_keys`); the
//! primary monitor is the one RandR marks, else the first. Rectangles are
//! pixels of the X11 root window.
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{Atom, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::monitor_keys::unique_keys;
use super::probe_timeout::timed_gated;
use crate::desktop::os::Monitor;

struct Found {
    name: String,
    rect: [i32; 4],
    primary: bool,
}

fn name_of_atom(conn: &RustConnection, atom: Atom) -> Option<String> {
    let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
    Some(String::from_utf8_lossy(&reply.name).into_owned())
}

fn output_name(conn: &RustConnection, output: randr::Output) -> Option<String> {
    let reply = conn.randr_get_output_info(output, 0).ok()?.reply().ok()?;
    Some(String::from_utf8_lossy(&reply.name).into_owned())
}

/// RandR 1.5 monitors; the name of the first output, else the monitor's own.
fn randr_monitors(conn: &RustConnection, root: Window) -> Option<Vec<Found>> {
    let reply = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
    let mut found = Vec::new();
    for monitor in reply.monitors {
        let name = monitor
            .outputs
            .first()
            .and_then(|output| output_name(conn, *output))
            .or_else(|| name_of_atom(conn, monitor.name))
            .unwrap_or_default();
        let (x, y) = (i32::from(monitor.x), i32::from(monitor.y));
        found.push(Found {
            name,
            rect: [
                x,
                y,
                x + i32::from(monitor.width),
                y + i32::from(monitor.height),
            ],
            primary: monitor.primary,
        });
    }
    Some(found)
}

/// RandR before 1.5: the CRTCs that show something, named after their first
/// output.
fn crtc_monitors(conn: &RustConnection, root: Window) -> Option<Vec<Found>> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()?
        .reply()
        .ok()?;
    let primary = conn
        .randr_get_output_primary(root)
        .ok()?
        .reply()
        .ok()?
        .output;
    let mut found = Vec::new();
    for crtc in resources.crtcs {
        let info = conn
            .randr_get_crtc_info(crtc, resources.config_timestamp)
            .ok()?
            .reply()
            .ok()?;
        let Some(output) = info.outputs.first().copied() else {
            continue;
        };
        if info.width == 0 || info.height == 0 {
            continue;
        }
        let (x, y) = (i32::from(info.x), i32::from(info.y));
        found.push(Found {
            name: output_name(conn, output).unwrap_or_default(),
            rect: [x, y, x + i32::from(info.width), y + i32::from(info.height)],
            primary: output == primary,
        });
    }
    Some(found)
}

/// The monitors of the server behind `conn`. Without RandR the root window is
/// the one monitor `screen`.
pub(super) fn list_monitors(conn: &RustConnection, root: Window, screen: usize) -> Vec<Monitor> {
    let mut found = randr_monitors(conn, root)
        .filter(|found| !found.is_empty())
        .or_else(|| crtc_monitors(conn, root).filter(|found| !found.is_empty()))
        .unwrap_or_default();
    if found.is_empty() {
        let Some(size) = conn.setup().roots.get(screen) else {
            return Vec::new();
        };
        found.push(Found {
            name: "screen".into(),
            rect: [
                0,
                0,
                i32::from(size.width_in_pixels),
                i32::from(size.height_in_pixels),
            ],
            primary: true,
        });
    }
    let names: Vec<String> = found.iter().map(|monitor| monitor.name.clone()).collect();
    let mut monitors: Vec<Monitor> = unique_keys(&names)
        .into_iter()
        .zip(found)
        .map(|(key, monitor)| Monitor {
            key,
            primary: monitor.primary,
            rect: Some(monitor.rect),
        })
        .collect();
    if !monitors.iter().any(|monitor| monitor.primary) {
        if let Some(first) = monitors.first_mut() {
            first.primary = true;
        }
    }
    monitors
}

fn read_monitors() -> Option<Vec<Monitor>> {
    let (conn, screen) = RustConnection::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    Some(list_monitors(&conn, root, screen))
}

/// The monitors of the X11 session (shell: once a second); none without a
/// connection.
pub(crate) fn monitors() -> Vec<Monitor> {
    timed_gated("x11-monitors", read_monitors)
        .flatten()
        .unwrap_or_default()
}

/// `(key, "<key> (<width>×<height>)")` of every monitor.
pub(crate) fn monitor_names() -> Vec<(String, String)> {
    monitors()
        .into_iter()
        .map(|monitor| {
            let label = match monitor.rect {
                Some([left, top, right, bottom]) => {
                    format!("{} ({}×{})", monitor.key, right - left, bottom - top)
                }
                None => monitor.key.clone(),
            };
            (monitor.key, label)
        })
        .collect()
}
