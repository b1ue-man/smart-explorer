//! X11 EWMH stacking + declared opaque regions intersected with XShape bounds.
//! Unknown opacity/shape, query races and Wayland stay conservatively visible.
use super::super::screen_visibility::WindowVisibility;
use eframe::egui::{self, Rect};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, GrabMode, MapState, ModMask};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

struct Native {
    conn: RustConnection,
    root: u32,
    window: u32,
    pause_keys: Vec<u8>,
}
thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }
fn reserve_pause(conn: &RustConnection, window: u32) -> Vec<u8> {
    let setup = conn.setup();
    let count = setup
        .max_keycode
        .saturating_sub(setup.min_keycode)
        .saturating_add(1);
    let Some(mapping) = conn
        .get_keyboard_mapping(setup.min_keycode, count)
        .ok()
        .and_then(|q| q.reply().ok())
    else {
        return Vec::new();
    };
    let stride = usize::from(mapping.keysyms_per_keycode);
    if stride == 0 {
        return Vec::new();
    }
    let mut keys = Vec::new();
    for (index, syms) in mapping.keysyms.chunks(stride).enumerate() {
        if !syms.contains(&0xff13) {
            continue;
        }
        let code = setup.min_keycode.saturating_add(index as u8);
        for extra in [
            ModMask::default(),
            ModMask::LOCK,
            ModMask::M2,
            ModMask::LOCK | ModMask::M2,
        ] {
            if let Ok(cookie) = conn.grab_key(
                false,
                window,
                ModMask::CONTROL | ModMask::M1 | extra,
                code,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            ) {
                let _ = cookie.check();
            }
        }
        keys.push(code);
    }
    let _ = conn.flush();
    keys
}

fn atom(conn: &RustConnection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(true, name)
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
        .filter(|a| *a != 0)
}
fn property(conn: &RustConnection, window: u32, atom: u32, ty: AtomEnum) -> Option<Vec<u32>> {
    let reply = conn
        .get_property(false, window, atom, ty, 0, 16_384)
        .ok()?
        .reply()
        .ok()?;
    if reply.bytes_after != 0 {
        return None;
    }
    Some(reply.value32()?.collect())
}
fn bounds(conn: &RustConnection, root: u32, window: u32) -> Option<Rect> {
    let geometry = conn.get_geometry(window).ok()?.reply().ok()?;
    let point = conn
        .translate_coordinates(window, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some(Rect::from_min_size(
        egui::pos2(f32::from(point.dst_x), f32::from(point.dst_y)),
        egui::vec2(f32::from(geometry.width), f32::from(geometry.height)),
    ))
}
fn shape(conn: &RustConnection, window: u32) -> Option<Vec<Rect>> {
    let reply = conn
        .shape_get_rectangles(window, SK::BOUNDING)
        .ok()?
        .reply()
        .ok()?;
    Some(
        reply
            .rectangles
            .into_iter()
            .map(|part| {
                Rect::from_min_size(
                    egui::pos2(f32::from(part.x), f32::from(part.y)),
                    egui::vec2(f32::from(part.width), f32::from(part.height)),
                )
            })
            .collect(),
    )
}
fn pause(conn: &RustConnection, ctx: &egui::Context) -> bool {
    if !ctx.input(|i| i.focused && i.modifiers.ctrl && i.modifiers.alt) {
        return false;
    }
    let setup = conn.setup();
    let count = setup
        .max_keycode
        .saturating_sub(setup.min_keycode)
        .saturating_add(1);
    let Some(mapping) = conn
        .get_keyboard_mapping(setup.min_keycode, count)
        .ok()
        .and_then(|q| q.reply().ok())
    else {
        return false;
    };
    let Some(keys) = conn.query_keymap().ok().and_then(|q| q.reply().ok()) else {
        return false;
    };
    let stride = usize::from(mapping.keysyms_per_keycode);
    if stride == 0 {
        return false;
    }
    mapping
        .keysyms
        .chunks(stride)
        .enumerate()
        .any(|(index, syms)| {
            let code = usize::from(setup.min_keycode) + index;
            syms.contains(&0xff13) && keys.keys[code / 8] & (1 << (code % 8)) != 0
        })
}
pub(crate) fn sample(frame: &eframe::Frame, ctx: &egui::Context) -> WindowVisibility {
    let mut result = WindowVisibility {
        minimized: ctx.input(|i| i.viewport().minimized == Some(true)),
        ..Default::default()
    };
    if result.minimized {
        result.shown = Some(false);
        return result;
    }
    let Ok(handle) = frame.window_handle() else {
        return result;
    };
    let window = match handle.as_raw() {
        RawWindowHandle::Xlib(raw) => raw.window as u32,
        RawWindowHandle::Xcb(raw) => raw.window.get(),
        // Wayland has no trustworthy global Z order. Do not guess from focus.
        _ => return result,
    };
    NATIVE.with(|slot| {
        let mut stored = slot.borrow_mut();
        if stored.is_none() {
            let Ok((conn, screen)) = x11rb::connect(None) else {
                return result;
            };
            let root = conn.setup().roots[screen].root;
            let pause_keys = reserve_pause(&conn, window);
            *stored = Some(Native {
                conn,
                root,
                window,
                pause_keys,
            });
        }
        let Some(native) = stored.as_mut() else {
            return result;
        };
        if native.window != window {
            native.pause_keys = reserve_pause(&native.conn, window);
            native.window = window;
        }
        let mut exit = false;
        while let Ok(Some(event)) = native.conn.poll_for_event() {
            if let Event::KeyPress(key) = event {
                exit |= native.pause_keys.contains(&key.detail);
            }
        }
        sample_x11(&native.conn, native.root, window, ctx, result, exit)
    })
}
fn sample_x11(
    conn: &RustConnection,
    root: u32,
    window: u32,
    ctx: &egui::Context,
    mut result: WindowVisibility,
    exit: bool,
) -> WindowVisibility {
    result.exit_chord = exit || pause(conn, ctx);
    let Ok(cookie) = conn.get_window_attributes(window) else {
        return result;
    };
    let Ok(attributes) = cookie.reply() else {
        return result;
    };
    if attributes.map_state != MapState::VIEWABLE {
        result.shown = Some(false);
        return result;
    }
    let Some(stack_atom) = atom(&conn, b"_NET_CLIENT_LIST_STACKING") else {
        return result;
    };
    let Some(opaque_atom) = atom(&conn, b"_NET_WM_OPAQUE_REGION") else {
        return result;
    };
    let opacity_atom = atom(&conn, b"_NET_WM_WINDOW_OPACITY");
    let Some(stack) = property(&conn, root, stack_atom, AtomEnum::WINDOW) else {
        return result;
    };
    let Some(index) = stack.iter().position(|&w| w == window) else {
        return result;
    };
    let Some(area) = bounds(&conn, root, window) else {
        return result;
    };
    let mut visible = vec![area];
    let mut observed = Vec::new();
    for &cover in &stack[index + 1..] {
        let Some(attrs) = conn
            .get_window_attributes(cover)
            .ok()
            .and_then(|q| q.reply().ok())
        else {
            continue;
        };
        if attrs.map_state != MapState::VIEWABLE {
            continue;
        }
        let opacity = opacity_atom.and_then(|a| property(conn, cover, a, AtomEnum::CARDINAL));
        if opacity
            .as_ref()
            .is_some_and(|values| values.as_slice() != [u32::MAX])
        {
            continue;
        }
        let Some(opaque) = property(&conn, cover, opaque_atom, AtomEnum::CARDINAL) else {
            continue;
        };
        if opaque.len() % 4 != 0 {
            continue;
        }
        let Some(rect) = bounds(&conn, root, cover) else {
            continue;
        };
        let Some(bounding) = shape(conn, cover) else {
            continue;
        };
        for chunk in opaque.chunks_exact(4) {
            let local = Rect::from_min_size(
                egui::pos2(chunk[0] as f32, chunk[1] as f32),
                egui::vec2(chunk[2] as f32, chunk[3] as f32),
            );
            // EWMH 5.18: an opaque hint never fills holes in the bounding shape.
            for region in &bounding {
                let occluded = local
                    .intersect(*region)
                    .translate(rect.min.to_vec2())
                    .intersect(rect);
                if occluded.is_positive() {
                    super::subtract(&mut visible, occluded);
                }
            }
        }
        observed.push((cover, rect, opaque, opacity, bounding));
    }
    if property(&conn, root, stack_atom, AtomEnum::WINDOW).as_ref() != Some(&stack)
        || bounds(&conn, root, window) != Some(area)
        || observed.iter().any(|(w, r, opaque, opacity, bounding)| {
            bounds(conn, root, *w) != Some(*r)
                || property(conn, *w, opaque_atom, AtomEnum::CARDINAL).as_ref() != Some(opaque)
                || shape(conn, *w).as_ref() != Some(bounding)
                || opacity_atom
                    .and_then(|a| property(conn, *w, a, AtomEnum::CARDINAL))
                    .as_ref()
                    != opacity.as_ref()
                || conn
                    .get_window_attributes(*w)
                    .ok()
                    .and_then(|q| q.reply().ok())
                    .is_none_or(|a| a.map_state != MapState::VIEWABLE)
        })
    {
        return result;
    }
    result.rects = visible
        .into_iter()
        .map(|rect| rect.translate(-area.min.to_vec2()))
        .collect();
    result.shown = Some(!result.rects.is_empty());
    result
}
