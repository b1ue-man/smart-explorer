use super::prelude::*;
use super::*;

pub(in crate::app) const TM_MIN: f32 = 3.0; // skip cells smaller than this
pub(in crate::app) const TM_RECURSE: f32 = 38.0; // only recurse into folders at least this big
pub(in crate::app) const TM_HEADER: f32 = 15.0; // folder header strip height
pub(in crate::app) const TM_MAXDEPTH: usize = 14;
pub(in crate::app) const TM_MAXCELLS: usize = 80_000;

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
pub(in crate::app) fn nested_treemap(
    rect: egui::Rect,
    node: &crate::analytics::SizeNode,
    base: &str,
    depth: usize,
    group: Option<Color32>,
    cells: &mut Vec<TmCell>,
) {
    if cells.len() >= TM_MAXCELLS
        || rect.width() < TM_MIN
        || rect.height() < TM_MIN
        || node.children.is_empty()
    {
        return;
    }
    let weights: Vec<f64> = node.children.iter().map(|c| c.size.max(1) as f64).collect();
    let rects = treemap_layout(&weights, rect);
    // Palette by size-rank: squarify places cells in size order, so consecutive
    // ranks land side-by-side → adjacent groups get different hues.
    let mut rank = vec![0usize; node.children.len()];
    {
        let mut order: Vec<usize> = (0..node.children.len()).collect();
        order.sort_by(|&a, &b| node.children[b].size.cmp(&node.children[a].size));
        for (r, &idx) in order.iter().enumerate() {
            rank[idx] = r;
        }
    }
    for (i, (c, r)) in node.children.iter().zip(&rects).enumerate() {
        if r.width() < TM_MIN || r.height() < TM_MIN {
            continue;
        }
        // Focus-level children seed a hue by rank; deeper cells inherit it.
        let gcol = group.unwrap_or(TM_PALETTE[rank[i] % TM_PALETTE.len()]);
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
                is_dir: true,
                container: true,
                color: gcol,
            });
            let inner = egui::Rect::from_min_max(
                egui::pos2(r.min.x + 1.5, r.min.y + TM_HEADER),
                egui::pos2(r.max.x - 1.5, r.max.y - 1.5),
            );
            nested_treemap(inner, c, &path, depth + 1, Some(gcol), cells);
        } else {
            cells.push(TmCell {
                rect: *r,
                name: c.name.to_string(),
                path,
                size: c.size,
                is_dir: c.is_dir,
                container: false,
                color: gcol,
            });
        }
    }
}
