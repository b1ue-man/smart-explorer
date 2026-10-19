//! The monitors of an X11 server (SCR-M5, U19-3): the active monitors of
//! RandR 1.5, else the CRTCs of the current screen resources with the name
//! of their first output. Source ids are `monitor:<output name>` with every
//! character outside visible ASCII (0x21..=0x7E) replaced by `_`; position
//! and size are pixels of the X11 root window.
use std::io;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, Window};

use crate::screen::core::window_list::{MonitorInfo, MAX_ID_BYTES};

/// Id of a monitor named `name`: visible ASCII only, at most 64 bytes; an
/// empty name becomes `monitor`.
pub(crate) fn monitor_id(name: &str) -> String {
    let mut id: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() { c } else { '_' })
        .collect();
    if id.is_empty() {
        id.push_str("monitor");
    }
    id.truncate(MAX_ID_BYTES);
    id
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("x11: {error}"))
}

/// The monitors of the server behind `conn`; the primary one is marked, and
/// when the server names none the first one is primary.
pub(crate) fn list<C: Connection>(conn: &C, root: Window) -> io::Result<Vec<MonitorInfo>> {
    let mut monitors = match randr_monitors(conn, root) {
        Ok(monitors) if !monitors.is_empty() => monitors,
        _ => match crtc_monitors(conn, root) {
            Ok(monitors) if !monitors.is_empty() => monitors,
            _ => root_monitor(conn, root)?,
        },
    };
    if !monitors.iter().any(|monitor| monitor.primary) {
        if let Some(first) = monitors.first_mut() {
            first.primary = true;
        }
    }
    Ok(monitors)
}

fn randr_monitors<C: Connection>(conn: &C, root: Window) -> io::Result<Vec<MonitorInfo>> {
    let reply = conn
        .randr_get_monitors(root, true)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let mut monitors = Vec::new();
    for monitor in reply.monitors {
        let name = conn
            .get_atom_name(monitor.name)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let name = String::from_utf8_lossy(&name.name).into_owned();
        monitors.push(MonitorInfo {
            id: monitor_id(&name),
            name,
            x: i32::from(monitor.x),
            y: i32::from(monitor.y),
            width: u32::from(monitor.width),
            height: u32::from(monitor.height),
            primary: monitor.primary,
        });
    }
    Ok(monitors)
}

/// RandR before 1.5: the CRTCs that show something, named after their first
/// output; the primary output of the server marks its CRTC's monitor.
fn crtc_monitors<C: Connection>(conn: &C, root: Window) -> io::Result<Vec<MonitorInfo>> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let primary_output = conn
        .randr_get_output_primary(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?
        .output;
    let mut monitors = Vec::new();
    for crtc in resources.crtcs {
        let info = conn
            .randr_get_crtc_info(crtc, resources.config_timestamp)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let Some(output) = info.outputs.first().copied() else {
            continue;
        };
        if info.width == 0 || info.height == 0 {
            continue;
        }
        let output_info = conn
            .randr_get_output_info(output, resources.config_timestamp)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let name = String::from_utf8_lossy(&output_info.name).into_owned();
        monitors.push(MonitorInfo {
            id: monitor_id(&name),
            name,
            x: i32::from(info.x),
            y: i32::from(info.y),
            width: u32::from(info.width),
            height: u32::from(info.height),
            primary: output == primary_output,
        });
    }
    Ok(monitors)
}

/// A server without RandR: the root window is the one monitor.
fn root_monitor<C: Connection>(conn: &C, root: Window) -> io::Result<Vec<MonitorInfo>> {
    let geometry = conn
        .get_geometry(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok(vec![MonitorInfo {
        id: "root".into(),
        name: "root".into(),
        x: 0,
        y: 0,
        width: u32::from(geometry.width),
        height: u32::from(geometry.height),
        primary: true,
    }])
}
