//! Native viewer observations; no OS discovery in the portable visibility model.
#[cfg(windows)]
#[path = "../windows/screen_visibility.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "../linux_os/screen_visibility.rs"]
mod platform;
pub(crate) use platform::sample;

use eframe::egui::Rect;
/// Subtract only established opaque rectangles; uncertain overlays never
/// remove pixels. Bound complexity by falling back to the previous superset.
pub(crate) fn subtract(rects: &mut Vec<Rect>, cover: Rect) {
    let mut out = Vec::new();
    for rect in rects.iter().copied() {
        let cut = rect.intersect(cover);
        if !cut.is_positive() {
            out.push(rect);
            continue;
        }
        for part in [
            Rect::from_min_max(rect.min, eframe::egui::pos2(rect.max.x, cut.min.y)),
            Rect::from_min_max(eframe::egui::pos2(rect.min.x, cut.max.y), rect.max),
            Rect::from_min_max(
                eframe::egui::pos2(rect.min.x, cut.min.y),
                eframe::egui::pos2(cut.min.x, cut.max.y),
            ),
            Rect::from_min_max(
                eframe::egui::pos2(cut.max.x, cut.min.y),
                eframe::egui::pos2(rect.max.x, cut.max.y),
            ),
        ] {
            if part.is_positive() {
                out.push(part);
            }
        }
        if out.len() > 128 {
            return;
        }
    }
    *rects = out;
}
