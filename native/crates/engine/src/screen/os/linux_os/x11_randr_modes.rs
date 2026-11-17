//! se's display modes on an X server (plan 2026-11-17 P5 stage 1, decision
//! E3): with RandR 1.2 outputs that take modes (the Xorg drivers `dummy`
//! and `modesetting`, not VESA, whose server emulates one output `default`)
//! se creates the mode `SE_<w>x<h>` with CVT reduced-blanking timings, adds
//! it to the output that shows the desktop, switches the CRTC and the
//! screen size to it while the server is grabbed (as Chrome Remote Desktop's
//! `desktop_resizer_x11.cc` does) and removes its older modes. Other active
//! monitors keep their place: the screen covers them. A physical monitor
//! gets its preferred mode back after the last session (`restore`). The
//! capture
//! (`x11_capture_target.rs`) reads the monitor area before every picture,
//! so the next picture has the new size (new reference generation, new
//! encoder).
use std::io;

use x11rb::connection::Connection;
use x11rb::protocol::randr::{
    Connection as OutputState, ConnectionExt as _, Crtc, Mode, ModeFlag, ModeInfo, Output,
    Rotation, SetConfig,
};
use x11rb::protocol::xproto::{ConnectionExt as _, Window};

use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display::{cvt_rb, is_own_mode, mode_name, REFRESH_HZ};
use crate::screen::core::virtual_display_host::DisplayFacts;

/// The timestamp `CurrentTime` of the X protocol.
const CURRENT_TIME: u32 = 0;
/// The output a server without a RandR 1.2 driver emulates.
const EMULATED_OUTPUT: &str = "default";
/// Millimetres per 10 inches: sizes are reported at 96 dpi.
const MM_PER_10_INCH: u64 = 254;

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("randr: {error}"))
}

/// Whether a server with RandR `version` and these output names takes
/// se's modes: RandR 1.2 or newer and an output other than the emulated
/// `default` of a driver without RandR 1.2 (VESA, fbdev).
pub fn takes_modes(version: (u32, u32), outputs: &[String]) -> bool {
    version >= (1, 2) && outputs.iter().any(|name| name != EMULATED_OUTPUT)
}

/// The RandR facts of the server behind `conn`.
pub fn probe<C: Connection>(conn: &C, root: Window) -> io::Result<DisplayFacts> {
    let version = conn
        .randr_query_version(1, 5)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let range = conn
        .randr_get_screen_size_range(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let geometry = conn
        .get_geometry(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let resources = conn
        .randr_get_screen_resources_current(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let mut names = Vec::with_capacity(resources.outputs.len());
    let mut connected = Vec::new();
    for output in &resources.outputs {
        let info = conn
            .randr_get_output_info(*output, resources.config_timestamp)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        let name = String::from_utf8_lossy(&info.name).into_owned();
        if info.connection == OutputState::CONNECTED {
            connected.push(name.clone());
        }
        names.push(name);
    }
    Ok(DisplayFacts {
        modes: takes_modes((version.major_version, version.minor_version), &names),
        max_width: u32::from(range.max_width),
        max_height: u32::from(range.max_height),
        current: FrameSize::new(u32::from(geometry.width), u32::from(geometry.height)),
        outputs: connected,
    })
}

/// The output that shows the desktop, its CRTC and its modes.
struct Target {
    output: Output,
    crtc: Crtc,
    modes: Vec<Mode>,
    /// The first preferred mode of the output (its monitor's native one).
    preferred: Option<Mode>,
}

/// The primary output when it drives a CRTC, else the first connected one
/// with a CRTC, else the first connected one that can get a CRTC.
fn target_output<C: Connection>(
    conn: &C,
    root: Window,
    outputs: &[Output],
    config_timestamp: u32,
) -> io::Result<Target> {
    let primary = conn
        .randr_get_output_primary(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?
        .output;
    let mut driving = None;
    let mut possible = None;
    for output in outputs {
        let info = conn
            .randr_get_output_info(*output, config_timestamp)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        if info.connection != OutputState::CONNECTED
            || String::from_utf8_lossy(&info.name) == EMULATED_OUTPUT
        {
            continue;
        }
        let preferred = info
            .modes
            .first()
            .copied()
            .filter(|_| info.num_preferred > 0);
        if info.crtc != 0 {
            let found = Target {
                output: *output,
                crtc: info.crtc,
                modes: info.modes.clone(),
                preferred,
            };
            if *output == primary {
                return Ok(found);
            }
            driving.get_or_insert(found);
        } else if let Some(crtc) = info.crtcs.first() {
            possible.get_or_insert(Target {
                output: *output,
                crtc: *crtc,
                modes: info.modes.clone(),
                preferred,
            });
        }
    }
    driving
        .or(possible)
        .ok_or_else(|| failed("no connected output with a CRTC"))
}

/// The modes of the screen resources with their names.
fn named_modes(modes: &[ModeInfo], names: &[u8]) -> Vec<(Mode, String)> {
    let mut at = 0usize;
    let mut named = Vec::with_capacity(modes.len());
    for mode in modes {
        let end = (at + usize::from(mode.name_len)).min(names.len());
        named.push((
            mode.id,
            String::from_utf8_lossy(&names[at..end]).into_owned(),
        ));
        at = end;
    }
    named
}

fn side(value: u32) -> io::Result<u16> {
    u16::try_from(value).map_err(|_| failed("display mode side exceeds 65535"))
}

/// The X11 mode of `size` (CVT reduced blanking at 60 Hz).
fn mode_info(size: FrameSize, name: &str) -> io::Result<ModeInfo> {
    let timing = cvt_rb(size.width, size.height, REFRESH_HZ)?;
    Ok(ModeInfo {
        id: 0,
        width: side(timing.hdisplay)?,
        height: side(timing.vdisplay)?,
        dot_clock: timing.clock_khz.saturating_mul(1000),
        hsync_start: side(timing.hsync_start)?,
        hsync_end: side(timing.hsync_end)?,
        htotal: side(timing.htotal)?,
        hskew: 0,
        vsync_start: side(timing.vsync_start)?,
        vsync_end: side(timing.vsync_end)?,
        vtotal: side(timing.vtotal)?,
        name_len: u16::try_from(name.len()).map_err(failed)?,
        mode_flags: ModeFlag::HSYNC_POSITIVE | ModeFlag::VSYNC_NEGATIVE,
    })
}

fn millimetres(pixels: u32) -> u32 {
    (u64::from(pixels) * MM_PER_10_INCH / 960) as u32
}

/// Where the target CRTC shows (its place, or the origin when it is off)
/// and the screen size that covers it at `size` and every other active
/// CRTC.
fn layout<C: Connection>(
    conn: &C,
    crtcs: &[Crtc],
    target: Crtc,
    size: FrameSize,
    config_timestamp: u32,
) -> io::Result<((i16, i16), FrameSize)> {
    let mut place = (0i16, 0i16);
    let mut others = Vec::new();
    for crtc in crtcs {
        let info = conn
            .randr_get_crtc_info(*crtc, config_timestamp)
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        if *crtc == target {
            if info.mode != 0 {
                place = (info.x, info.y);
            }
        } else if info.mode != 0 {
            others.push((info.x, info.y, info.width, info.height));
        }
    }
    let reach =
        |start: i16, length: u32| u32::try_from(i64::from(start).max(0)).unwrap_or(0) + length;
    let mut width = reach(place.0, size.width);
    let mut height = reach(place.1, size.height);
    for (x, y, w, h) in others {
        width = width.max(reach(x, u32::from(w)));
        height = height.max(reach(y, u32::from(h)));
    }
    Ok((place, FrameSize::new(width, height)))
}

/// Switches the desktop to `size`: mode, CRTC and screen size, the server
/// grabbed meanwhile; then se's older modes go. The size the CRTC shows
/// afterwards.
pub fn apply<C: Connection>(conn: &C, root: Window, size: FrameSize) -> io::Result<FrameSize> {
    let name = mode_name(size);
    let resources = conn
        .randr_get_screen_resources_current(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let target = target_output(conn, root, &resources.outputs, resources.config_timestamp)?;
    let known = named_modes(&resources.modes, &resources.names);
    let mode = match known.iter().find(|(_, known)| *known == name) {
        Some((id, _)) => *id,
        None => {
            conn.randr_create_mode(root, mode_info(size, &name)?, name.as_bytes())
                .map_err(failed)?
                .reply()
                .map_err(failed)?
                .mode
        }
    };
    if !target.modes.contains(&mode) {
        conn.randr_add_output_mode(target.output, mode)
            .map_err(failed)?
            .check()
            .map_err(failed)?;
    }
    // Adding a mode moves the configuration timestamp the CRTC calls name.
    let resources = conn
        .randr_get_screen_resources_current(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    switch_grabbed(
        conn,
        root,
        &target,
        mode,
        size,
        &resources.crtcs,
        resources.config_timestamp,
    )?;
    remove_own_modes(conn, &target, &known, mode);
    let shown = conn
        .randr_get_crtc_info(target.crtc, CURRENT_TIME)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    if shown.width == 0 || shown.height == 0 {
        return Err(failed("the CRTC shows nothing after the change"));
    }
    Ok(FrameSize::new(
        u32::from(shown.width),
        u32::from(shown.height),
    ))
}

/// Gives the output that shows the desktop its preferred mode back when it
/// shows one of se's modes (a physical monitor after a viewer's „Eigener
/// (virtuell)“) and removes se's modes; `None` when it shows another mode.
/// The size it shows afterwards.
pub fn restore<C: Connection>(conn: &C, root: Window) -> io::Result<Option<FrameSize>> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let timestamp = resources.config_timestamp;
    let target = target_output(conn, root, &resources.outputs, timestamp)?;
    let known = named_modes(&resources.modes, &resources.names);
    let shown = conn
        .randr_get_crtc_info(target.crtc, timestamp)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let own = known
        .iter()
        .any(|(id, name)| *id == shown.mode && is_own_mode(name));
    let Some(preferred) = target.preferred.filter(|_| own) else {
        return Ok(None);
    };
    let info = resources
        .modes
        .iter()
        .find(|mode| mode.id == preferred)
        .ok_or_else(|| failed("the preferred mode is not in the screen resources"))?;
    let size = FrameSize::new(u32::from(info.width), u32::from(info.height));
    switch_grabbed(
        conn,
        root,
        &target,
        preferred,
        size,
        &resources.crtcs,
        timestamp,
    )?;
    remove_own_modes(conn, &target, &known, preferred);
    Ok(Some(size))
}

/// `switch` with the server grabbed, as one change for every other client.
fn switch_grabbed<C: Connection>(
    conn: &C,
    root: Window,
    target: &Target,
    mode: Mode,
    size: FrameSize,
    crtcs: &[Crtc],
    config_timestamp: u32,
) -> io::Result<()> {
    let (place, screen) = layout(conn, crtcs, target.crtc, size, config_timestamp)?;
    conn.grab_server()
        .map_err(failed)?
        .check()
        .map_err(failed)?;
    let switched = switch(conn, root, target, mode, place, screen, config_timestamp);
    let released = conn
        .ungrab_server()
        .map_err(failed)
        .and_then(|cookie| cookie.check().map_err(failed));
    conn.flush().map_err(failed)?;
    switched?;
    released
}

/// Removes se's modes other than `keep` from the output and the server; a
/// mode still bound elsewhere stays, which is no failure.
fn remove_own_modes<C: Connection>(
    conn: &C,
    target: &Target,
    known: &[(Mode, String)],
    keep: Mode,
) {
    for (old, name) in known {
        if *old != keep && is_own_mode(name) {
            if let Ok(cookie) = conn.randr_delete_output_mode(target.output, *old) {
                let _ = cookie.check();
            }
            if let Ok(cookie) = conn.randr_destroy_mode(*old) {
                let _ = cookie.check();
            }
        }
    }
}

/// The CRTC off, the screen to the new size, the CRTC on with the mode.
fn switch<C: Connection>(
    conn: &C,
    root: Window,
    target: &Target,
    mode: Mode,
    place: (i16, i16),
    screen: FrameSize,
    config_timestamp: u32,
) -> io::Result<()> {
    let set = |mode: Mode, place: (i16, i16), outputs: &[Output]| -> io::Result<()> {
        let reply = conn
            .randr_set_crtc_config(
                target.crtc,
                CURRENT_TIME,
                config_timestamp,
                place.0,
                place.1,
                mode,
                Rotation::ROTATE0,
                outputs,
            )
            .map_err(failed)?
            .reply()
            .map_err(failed)?;
        if reply.status == SetConfig::SUCCESS {
            Ok(())
        } else {
            Err(failed(format!(
                "SetCrtcConfig refused ({})",
                u8::from(reply.status)
            )))
        }
    };
    set(0, (0, 0), &[])?;
    conn.randr_set_screen_size(
        root,
        side(screen.width)?,
        side(screen.height)?,
        millimetres(screen.width),
        millimetres(screen.height),
    )
    .map_err(failed)?
    .check()
    .map_err(failed)?;
    set(mode, place, &[target.output])
}
