use super::prelude::*;
use super::*;

pub(in crate::app) const TM_MIN: f32 = 3.0; // skip cells smaller than this
pub(in crate::app) const TM_RECURSE: f32 = 38.0; // only recurse into folders at least this big
pub(in crate::app) const TM_HEADER: f32 = 20.0; // room for readable chart labels
pub(in crate::app) const TM_MAXDEPTH: usize = 14;
pub(in crate::app) const TM_MAXCELLS: usize = 80_000;

pub(in crate::app) fn treemap_needs_layout(
    has_node: bool,
    cells_empty: bool,
    cached: egui::Rect,
    current: egui::Rect,
) -> bool {
    // Cells retain absolute coordinates. Opening diagnostics or moving the
    // window can change the origin without changing the available size.
    has_node && (cells_empty || cached != current)
}

/// Distinct, slightly muted hues for treemap groups. Each immediate child of the
/// focus picks the next palette colour (so adjacent groups differ); the whole
/// subtree inherits that colour — you see the grouping, not file types.
pub(in crate::app) const TM_PALETTE: [Color32; 10] = [
    Color32::from_rgb(0x5b, 0x8f, 0xc9), // blue
    Color32::from_rgb(0xe0, 0x8a, 0x3c), // orange
    Color32::from_rgb(0x6f, 0xb5, 0x6a), // green
    Color32::from_rgb(0xc9, 0x5f, 0x5f), // red
    Color32::from_rgb(0x9b, 0x7c, 0xc9), // purple
    Color32::from_rgb(0x4f, 0xb0, 0xb0), // teal
    Color32::from_rgb(0xc9, 0xa8, 0x4f), // gold
    Color32::from_rgb(0xc9, 0x6f, 0xa8), // pink
    Color32::from_rgb(0x7f, 0x9b, 0x55), // olive
    Color32::from_rgb(0x6f, 0x8a, 0xa8), // slate
];

/// Lay out `node`'s children into `rect` as a squarified treemap, recursing into
/// folders that are big enough (WizTree-style nested view). Collects paintable
/// `TmCell`s; the parent is responsible for painting them. `group` is the hue
/// inherited from the focus-level ancestor (None at the focus level itself).
/// Areas and ranks follow `measure` („Belegt“ or „Größe“); the children are
/// already sorted by it (`MeasureView::sort_tree`), so a child's index is its
/// rank. Labels are made here once, not on every painted frame.
pub(in crate::app) fn nested_treemap(
    rect: egui::Rect,
    node: &crate::analytics::SizeNode,
    base: &str,
    depth: usize,
    group: Option<Color32>,
    measure: super::analytics_measure::MeasureView,
    cells: &mut Vec<TmCell>,
) {
    if cells.len() >= TM_MAXCELLS
        || rect.width() < TM_MIN
        || rect.height() < TM_MIN
        || node.children.is_empty()
    {
        return;
    }
    let weights: Vec<f64> = node
        .children
        .iter()
        .map(|c| measure.weight(c).max(1) as f64)
        .collect();
    let rects = treemap_layout(&weights, rect);
    // Palette by rank (the sorted index): squarify places cells in that
    // order, so consecutive ranks land side-by-side → adjacent groups differ.
    for (rank, (c, r)) in node.children.iter().zip(&rects).enumerate() {
        if r.width() < TM_MIN || r.height() < TM_MIN {
            continue;
        }
        // Focus-level children seed a hue by rank; deeper cells inherit it.
        let gcol = group.unwrap_or(TM_PALETTE[rank % TM_PALETTE.len()]);
        let path = format!("{}/{}", base, c.name);
        let recurse = c.is_dir
            && !c.children.is_empty()
            && depth < TM_MAXDEPTH
            && r.width() >= TM_RECURSE
            && r.height() >= TM_RECURSE + TM_HEADER;
        if recurse {
            cells.push(TmCell {
                rect: *r,
                name: c.name.to_string(),
                path: path.clone(),
                size: c.size,
                allocated: c.allocated,
                weight: measure.weight(c),
                label: cell_label(*r, true, &c.name, measure.weight(c)),
                is_dir: true,
                container: true,
                color: gcol,
            });
            let inner = egui::Rect::from_min_max(
                egui::pos2(r.min.x + 1.5, r.min.y + TM_HEADER),
                egui::pos2(r.max.x - 1.5, r.max.y - 1.5),
            );
            nested_treemap(inner, c, &path, depth + 1, Some(gcol), measure, cells);
        } else {
            cells.push(TmCell {
                rect: *r,
                name: c.name.to_string(),
                path,
                size: c.size,
                allocated: c.allocated,
                weight: measure.weight(c),
                label: cell_label(*r, false, &c.name, measure.weight(c)),
                is_dir: c.is_dir,
                container: false,
                color: gcol,
            });
        }
    }
}

/// Where a cell's label goes: a container's header, else the whole cell.
pub(in crate::app) fn label_rect(rect: egui::Rect, container: bool) -> egui::Rect {
    if container {
        egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.max.x, (rect.min.y + TM_HEADER).min(rect.max.y)),
        )
    } else {
        rect
    }
}

/// The label text of a cell that has room for one.
fn cell_label(rect: egui::Rect, container: bool, name: &str, weight: u64) -> Option<String> {
    let room = label_rect(rect, container);
    (room.width() > 40.0 && room.height() >= 17.0).then(|| {
        if container {
            format!("{name}  {}", format_bytes(weight))
        } else {
            format!("{name}\n{}", format_bytes(weight))
        }
    })
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[test]
    fn analytics_access_task_treemap_origin_change_invalidates_geometry() {
        let original = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 200.0));
        let shifted = original.translate(egui::vec2(0.0, 120.0));
        assert_eq!(original.size(), shifted.size());
        assert!(treemap_needs_layout(true, false, original, shifted));
        assert!(!treemap_needs_layout(true, false, original, original));
        assert!(treemap_needs_layout(true, true, original, original));
        assert!(!treemap_needs_layout(false, true, original, shifted));
    }
}
