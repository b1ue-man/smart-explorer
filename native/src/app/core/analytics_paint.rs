//! Common chart painting for ordinary and elevated read-only analysis.
use super::{app_models::TmCell, theme, treemap::TM_HEADER};
use crate::format::format_bytes;
use eframe::egui::{self, Color32};

pub(super) fn label_color(background: Color32) -> Color32 {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * linear(background.r()) + 0.7152 * linear(background.g())
        + 0.0722 * linear(background.b());
    // The crossover of black/white contrast guarantees at least 4.5:1.
    if luminance > 0.179 { Color32::BLACK } else { Color32::WHITE }
}

pub(super) fn paint(ui: &egui::Ui, rect: egui::Rect, cells: &[TmCell]) {
    let palette = theme::palette(ui);
    let dark = ui.visuals().dark_mode;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.surface);
    for cell in cells {
        let fill = if cell.container {
            if dark { cell.color.gamma_multiply(0.4) }
            else { egui::ecolor::tint_color_towards(cell.color, palette.surface) }
        } else { cell.color };
        painter.rect_filled(cell.rect, 2.0, fill);
        painter.rect_stroke(cell.rect, 2.0, egui::Stroke::new(1.0, palette.surface));
        let (label_rect, background) = if cell.container {
            let header = egui::Rect::from_min_max(cell.rect.min,
                egui::pos2(cell.rect.max.x, (cell.rect.min.y + TM_HEADER).min(cell.rect.max.y)));
            let background = if dark { cell.color.gamma_multiply(0.7) } else { fill };
            painter.rect_filled(header, 0.0, background);
            (header, background)
        } else { (cell.rect, fill) };
        if label_rect.width() > 40.0 && label_rect.height() >= 17.0 {
            let label = if cell.container { format!("{}  {}", cell.name, format_bytes(cell.size)) }
                else { format!("{}\n{}", cell.name, format_bytes(cell.size)) };
            painter.with_clip_rect(label_rect.shrink(2.0)).text(
                label_rect.min + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP,
                label, egui::FontId::proportional(12.0), label_color(background));
        }
    }
}
