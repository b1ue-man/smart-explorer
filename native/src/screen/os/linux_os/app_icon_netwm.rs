//! Class/PID-bound EWMH icon fallback and current window identity.
use super::*;

/// EWMH ARGB fallback only after the current window class proves this desktop mapping.
pub(super) fn window_icon(window: &WindowEntry, desktop: &DesktopIcon) -> Option<IconReply> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let xid: u32 = window.id.parse().ok()?;
    let (connection, _) = x11rb::rust_connection::RustConnection::connect(None).ok()?;
    let class = connection
        .get_property(false, xid, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if class.bytes_after != 0 {
        return None;
    }
    let class = super::super::x11_windows::app_of_wm_class(&class.value);
    let stem = desktop.id.strip_suffix(".desktop").unwrap_or(&desktop.id);
    if class != stem && desktop.startup_class.as_deref() != Some(class.as_str()) {
        return None;
    }
    if let Some(expected_pid) = window.pid {
        let atom = connection
            .intern_atom(false, b"_NET_WM_PID")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let property = connection
            .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        if property.value32()?.next() != Some(expected_pid) {
            return None;
        }
    }
    let atom = connection
        .intern_atom(false, b"_NET_WM_ICON")
        .ok()?
        .reply()
        .ok()?
        .atom;
    // The property itself is bounded before x11rb allocates its returned pixel words.
    let property = connection
        .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 65_538)
        .ok()?
        .reply()
        .ok()?;
    if property.bytes_after != 0 {
        return None;
    }
    let words: Vec<u32> = property.value32()?.collect();
    let mut at = 0usize;
    let mut chosen: Option<(u32, u32, &[u32])> = None;
    while let Some(header) = words.get(at..at.checked_add(2)?) {
        let (width, height) = (header[0], header[1]);
        let pixels = app_icon::checked_pixels(width, height).ok()?;
        let end = at.checked_add(2)?.checked_add(pixels)?;
        let content = words.get(at + 2..end)?;
        let distance = width.abs_diff(OUTPUT_SIDE) + height.abs_diff(OUTPUT_SIDE);
        if chosen
            .is_none_or(|(w, h, _)| distance < w.abs_diff(OUTPUT_SIDE) + h.abs_diff(OUTPUT_SIDE))
        {
            chosen = Some((width, height, content));
        }
        at = end;
    }
    if at != words.len() {
        return None;
    }
    let (width, height, content) = chosen?;
    let mut rgba = Vec::with_capacity(content.len() * 4);
    for pixel in content {
        rgba.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            *pixel as u8,
            (pixel >> 24) as u8,
        ]);
    }
    let png = super::super::app_icon_raster::rgba_png(width, height, &rgba).ok()?;
    IconReply::new(&desktop.id, &png).ok()
}

/// Refresh on every list: theme changes and changed app bytes produce new hashes.
pub(crate) fn window_identity(native: u64) -> io::Result<Option<String>> {
    let xid = u32::try_from(native)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "X11 window ID out of range"))?;
    let list = super::super::x11_windows::list()?;
    let id = super::super::x11_windows::window_id(xid);
    let Some(window) = list.windows.iter().find(|window| window.id == id) else {
        return Ok(None);
    };
    let dirs = super::super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let source = window.app_id.as_deref().unwrap_or(&window.app);
    Ok(desktop_for(&entries, source, true).map(|entry| entry.id.clone()))
}
