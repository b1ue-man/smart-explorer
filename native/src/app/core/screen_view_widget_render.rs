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
