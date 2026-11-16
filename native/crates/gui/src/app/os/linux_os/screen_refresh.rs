//! Refresh rate of the monitor the viewer's window is on (Linux).
//!
//! X11 (also XWayland): the CRTC under the centre of the active window
//! (`_NET_ACTIVE_WINDOW`, EWMH), else the fastest active CRTC; its RandR mode
//! gives dot clock / (htotal × vtotal), with the vertical total doubled for
//! double scan and halved for interlace, as `xrandr` computes `mode_refresh`.
//! Wayland: `wl_output` reports the current mode's refresh in mHz; a client
//! does not learn which output a window is on before it draws there, so the
//! fastest output counts [assumption: a viewer on a slower output gets more
//! pictures than it shows, never fewer].
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

pub fn refresh_millihertz() -> Option<u32> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        wayland_refresh().or_else(x11_refresh)
    } else {
        x11_refresh()
    }
}

/// Refresh of one RandR mode in mHz (`xrandr` `mode_refresh`).
fn mode_millihertz(mode: &randr::ModeInfo) -> Option<u32> {
    let flags = u32::from(mode.mode_flags);
    let mut vtotal = u64::from(mode.vtotal);
    if flags & u32::from(randr::ModeFlag::DOUBLE_SCAN) != 0 {
        vtotal *= 2;
    }
    if flags & u32::from(randr::ModeFlag::INTERLACE) != 0 {
        vtotal /= 2;
    }
    let pixels = u64::from(mode.htotal) * vtotal;
    if pixels == 0 {
        return None;
    }
    u32::try_from(u64::from(mode.dot_clock) * 1000 / pixels).ok()
}

/// Centre of the active window in root coordinates.
fn active_centre(conn: &RustConnection, root: Window) -> Option<(i32, i32)> {
    let atom = conn
        .intern_atom(true, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let window = reply.value32()?.next()?;
    let geometry = conn.get_geometry(window).ok()?.reply().ok()?;
    let origin = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((
        i32::from(origin.dst_x) + i32::from(geometry.width) / 2,
        i32::from(origin.dst_y) + i32::from(geometry.height) / 2,
    ))
}

fn x11_refresh() -> Option<u32> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()?
        .reply()
        .ok()?;
    let centre = active_centre(&conn, root);
    let mut fastest = None;
    for crtc in &resources.crtcs {
        let Some(info) = conn
            .randr_get_crtc_info(*crtc, resources.config_timestamp)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            continue;
        };
        let Some(refresh) = resources
            .modes
            .iter()
            .find(|mode| mode.id == info.mode)
            .and_then(mode_millihertz)
        else {
            continue;
        };
        let (x, y) = (i32::from(info.x), i32::from(info.y));
        let inside = centre.is_some_and(|(cx, cy)| {
            cx >= x && cy >= y && cx < x + i32::from(info.width) && cy < y + i32::from(info.height)
        });
        if inside {
            return Some(refresh);
        }
        fastest = fastest.max(Some(refresh));
    }
    fastest
}

#[derive(Default)]
struct Outputs {
    refresh: Vec<i32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Outputs {
    fn event(
        _state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _connection: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_output" {
                // Version 1 already sends the mode event with its refresh.
                registry.bind::<wl_output::WlOutput, _, _>(name, version.min(2), handle, ());
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Outputs {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode {
            flags: WEnum::Value(flags),
            refresh,
            ..
        } = event
        {
            if flags.contains(wl_output::Mode::Current) && refresh > 0 {
                state.refresh.push(refresh);
            }
        }
    }
}

fn wayland_refresh() -> Option<u32> {
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    let _registry = connection.display().get_registry(&handle, ());
    let mut outputs = Outputs::default();
    // The first round trip announces the globals, the second the modes.
    queue.roundtrip(&mut outputs).ok()?;
    queue.roundtrip(&mut outputs).ok()?;
    let fastest = outputs.refresh.into_iter().max()?;
    u32::try_from(fastest).ok()
}
