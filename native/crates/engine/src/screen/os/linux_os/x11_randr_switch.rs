//! The switch of the desktop's CRTC to a mode (`x11_randr_modes`): off,
//! the screen to the new size, on with the mode, the server grabbed; a
//! refused step restores the mode before (review 2026-11-19, Gemini 3/15).
use super::*;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;

/// `switch` with the server grabbed, as one change for every other client.
pub(super) fn switch_grabbed<C: Connection>(
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
    // Gemini 3/15: the CRTC's mode, place and outputs and the screen size
    // before, so a refused step leaves the monitor as it was, not dark.
    let before = conn
        .randr_get_crtc_info(target.crtc, config_timestamp)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let old_screen = conn
        .get_geometry(root)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let resize = |size: FrameSize| -> io::Result<()> {
        conn.randr_set_screen_size(
            root,
            side(size.width)?,
            side(size.height)?,
            millimetres(size.width),
            millimetres(size.height),
        )
        .map_err(failed)?
        .check()
        .map_err(failed)
    };
    set(0, (0, 0), &[])?;
    let switched = resize(screen).and_then(|()| set(mode, place, &[target.output]));
    if switched.is_err() && before.mode != 0 {
        let old = FrameSize::new(u32::from(old_screen.width), u32::from(old_screen.height));
        let back =
            resize(old).and_then(|()| set(before.mode, (before.x, before.y), &before.outputs));
        if let Err(error) = back {
            eprintln!("screen host process: the display's mode could not be restored: {error}");
        }
    }
    switched
}
