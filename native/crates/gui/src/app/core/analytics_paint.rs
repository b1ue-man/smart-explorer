//! Common chart painting for ordinary and elevated read-only analysis. The
//! fills of all cells (up to `TM_MAXCELLS`) are one mesh, built once per
//! layout and theme (`TreemapCache::mesh`); a frame only hands it over and
//! draws the labels made at layout time.
use super::analytics_view::{CachedMesh, TreemapCache};
use super::{app_models::TmCell, theme, treemap::label_rect};
use eframe::egui::{self, Color32};

pub(super) fn label_color(background: Color32) -> Color32 {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(background.r())
        + 0.7152 * linear(background.g())
        + 0.0722 * linear(background.b());
    // The crossover of black/white contrast guarantees at least 4.5:1.
    if luminance > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Fill of a cell and the background of its label.
fn colors(cell: &TmCell, dark: bool, surface: Color32) -> (Color32, Color32) {
    if !cell.container {
        return (cell.color, cell.color);
    }
    let fill = if dark {
        cell.color.gamma_multiply(0.4)
    } else {
        egui::ecolor::tint_color_towards(cell.color, surface)
    };
    let header = if dark {
        cell.color.gamma_multiply(0.7)
    } else {
        fill
    };
    (fill, header)
}

/// Every cell as a 1-pixel surface frame around its fill, a container with
/// its header band.
pub(super) fn build_mesh(cells: &[TmCell], dark: bool, surface: Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(cells.len() * 12);
    mesh.reserve_triangles(cells.len() * 6);
    for cell in cells {
        let (fill, header) = colors(cell, dark, surface);
        mesh.add_colored_rect(cell.rect, surface);
        let inner = cell.rect.shrink(0.5);
        if inner.is_positive() {
            mesh.add_colored_rect(inner, fill);
        }
        if cell.container {
            let band = label_rect(cell.rect, true).shrink(0.5);
            if band.is_positive() {
                mesh.add_colored_rect(band, header);
            }
        }
    }
    mesh
}

pub(super) fn paint(ui: &egui::Ui, rect: egui::Rect, cache: &TreemapCache) {
    let palette = theme::palette(ui);
    let dark = ui.visuals().dark_mode;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.surface);
    let mut cached = cache.mesh.borrow_mut();
    let stale = !matches!(
        cached.as_ref(),
        Some(built) if built.dark == dark && built.surface == palette.surface
    );
    if stale {
        *cached = Some(CachedMesh {
            dark,
            surface: palette.surface,
            mesh: build_mesh(&cache.cells, dark, palette.surface),
        });
    }
    if let Some(built) = cached.as_ref() {
        // egui 0.29 takes the mesh by value: a copy, no tessellation.
        painter.add(egui::Shape::mesh(built.mesh.clone()));
    }
    for cell in &cache.cells {
        let Some(label) = &cell.label else {
            continue;
        };
        let (_, background) = colors(cell, dark, palette.surface);
        let room = label_rect(cell.rect, cell.container);
        painter.with_clip_rect(room.shrink(2.0)).text(
            room.min + egui::vec2(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(12.0),
            label_color(background),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse_task_treemap_mesh_has_frame_fill_and_header_per_cell() {
        let cell = |container: bool| TmCell {
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 60.0)),
            name: "a".into(),
            path: "/a".into(),
            size: 1,
            allocated: 1,
            weight: 1,
            label: None,
            is_dir: container,
            container,
            color: Color32::from_rgb(0x5b, 0x8f, 0xc9),
        };
        let mesh = build_mesh(&[cell(false), cell(true)], true, Color32::BLACK);
        // Leaf: frame + fill; container: frame + fill + header (4 vertices each).
        assert_eq!(mesh.vertices.len(), 5 * 4);
        assert_eq!(mesh.indices.len(), 5 * 6);
        let empty = build_mesh(&[], false, Color32::WHITE);
        assert!(empty.vertices.is_empty());
    }
}
