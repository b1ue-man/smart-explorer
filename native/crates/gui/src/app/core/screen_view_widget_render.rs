//! Full-view rendering keeps the selected resident and its receipt together.
use super::{
    screen_input_map::Geometry, screen_session::ViewerSession, screen_texture_cpu::CpuTexture,
    screen_view_widget::Current,
};
use eframe::egui::{self, Align2, Color32, FontId};
pub(super) fn paint(
    current: Option<&Current>,
    cpu: &mut CpuTexture,
    session: &ViewerSession,
    painter: &egui::Painter,
    area: egui::Rect,
    geometry: &Geometry,
    native: bool,
) -> bool {
    let ctx = painter.ctx();
    let size = geometry.size;
    let dest = geometry.rect;
    let ppp = ctx.pixels_per_point();
    match current {
        Some(Current::Cpu(_)) => {
            cpu.prepare(
                ctx,
                if native {
                    [size.width as usize, size.height as usize]
                } else {
                    [
                        (dest.width() * ppp).round().max(1.0) as usize,
                        (dest.height() * ppp).round().max(1.0) as usize,
                    ]
                },
            );
            if cpu.failed() {
                painter.text(
                    area.center(),
                    Align2::CENTER_CENTER,
                    "Bildskalierung nicht verfügbar: Renderer- oder Speicherressource fehlt.",
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
            }
            cpu.paint(painter, dest, |frame| {
                session
                    .can_present(frame.clone())
                    .then(|| session.presented_callback(frame))
            })
        }
        Some(Current::Gpu(frame, stamp)) => super::screen_wgpu_yuv::paint(
            painter,
            dest,
            frame.clone(),
            stamp.clone(),
            native,
            cpu.slot(),
            |frame| {
                session
                    .can_present(frame.clone())
                    .then(|| session.presented_callback(frame))
            },
        ),
        None => false,
    }
}

/// A hint or the session's status over the picture (`ViewParams::hint`):
/// never part of the layout, so it does not change the picture area, which
/// would send a visibility report and maybe a new size to the host. Wrapped
/// left of the statistics field `stats`; the returned field is cut out of
/// the Direct3D 11 child like the statistics.
pub(super) fn hint(
    painter: &egui::Painter,
    area: egui::Rect,
    text: &str,
    stats: Option<egui::Rect>,
) -> egui::Rect {
    let right = stats.map_or(area.right(), |field| field.left());
    let wrap = (right - area.left() - 28.0).max(80.0);
    let colour = Color32::from_gray(230);
    let galley = painter.layout(text.to_owned(), FontId::proportional(14.0), colour, wrap);
    // Below the full-screen toolbar while it lies over the picture.
    let ctx = painter.ctx();
    let top = super::screen_viewer_app::toolbar_overlay_bottom(ctx)
        .map_or(area.top(), |b| b.max(area.top()));
    let top_left = egui::pos2(area.left() + 8.0, top + 8.0);
    let field = egui::Rect::from_min_size(top_left, galley.size() + egui::vec2(12.0, 8.0));
    painter.rect_filled(field, 4.0, Color32::from_black_alpha(170));
    painter.galley(top_left + egui::vec2(6.0, 4.0), galley, colour);
    field
}

/// Whether the picture has the keyboard (its own focus, no popup open; not
/// a text field of the toolbar, Gemini 10), and the modifiers that count.
/// When the keyboard leaves it, the keys the host holds are let go once and
/// the window's modifiers stay local (Codex 3/18). `owned`: last frame's.
pub(super) fn keyboard(
    ui: &egui::Ui,
    response: &egui::Response,
    mapper: &mut super::screen_input_map::InputMapper,
    owned: &mut bool,
) -> (bool, egui::Modifiers) {
    let popup = ui.memory(|memory| memory.any_popup_open());
    let keys = !popup && ui.memory(|memory| memory.has_focus(response.id));
    if *owned && !keys {
        mapper.release_all();
    }
    *owned = keys;
    let modifiers = match keys {
        true => ui.input(|input| input.modifiers),
        false => egui::Modifiers::NONE,
    };
    (keys, modifiers)
}

/// The window's events that belong to the picture while it steers (fix
/// 2026-11-19): a press, a move, the wheel or a touch only while the
/// pointer is over the picture and nothing covers it (a toolbar list or a
/// menu lying over the picture is the local user's), keys and text only
/// while the picture has the keyboard (`keyboard`); button releases always
/// pass, so what the host holds down is let go in order.
pub(super) fn owned_events(
    ui: &egui::Ui,
    response: &egui::Response,
    events: &[egui::Event],
    keys: bool,
) -> Vec<egui::Event> {
    use egui::{Event, TouchPhase};
    let popup = ui.memory(|memory| memory.any_popup_open());
    let pointer = !popup && (response.contains_pointer() || response.dragged());
    // The wheel scrolls where the pointer is, also before it moved since the
    // mapper began (Gemini 12: the mapper dropped the wheel until a move).
    let wheel = events
        .iter()
        .any(|event| matches!(event, Event::MouseWheel { .. }));
    let moved = events
        .iter()
        .any(|event| matches!(event, Event::PointerMoved(_)));
    let hover = ui.input(|input| input.pointer.hover_pos());
    let lead = hover
        .filter(|_| pointer && wheel && !moved)
        .map(Event::PointerMoved);
    lead.into_iter()
        .chain(
            events
                .iter()
                .filter(|event| match event {
                    Event::PointerButton { pressed, .. } => pointer || !pressed,
                    Event::PointerMoved(_) | Event::MouseWheel { .. } => pointer,
                    Event::Touch { phase, .. } => {
                        pointer || matches!(phase, TouchPhase::End | TouchPhase::Cancel)
                    }
                    // Without the keyboard no key at all, not even a release
                    // of one the host never got (`keyboard_owned`, Codex 3/18).
                    Event::Key { .. } => keys,
                    Event::Text(_) | Event::Ime(_) | Event::Copy | Event::Cut | Event::Paste(_) => {
                        keys
                    }
                    _ => true,
                })
                .cloned()
                .map(|event| match keys {
                    true => event,
                    // The window's modifiers stay local without the keyboard.
                    false => without_modifiers(event),
                }),
        )
        .collect()
}

/// `event` without the modifiers egui gave it (they sync the host's keys).
fn without_modifiers(mut event: egui::Event) -> egui::Event {
    match &mut event {
        egui::Event::PointerButton { modifiers, .. }
        | egui::Event::MouseWheel { modifiers, .. } => {
            *modifiers = egui::Modifiers::NONE;
        }
        _ => {}
    }
    event
}
