use super::prelude::*;

#[derive(Clone)]
pub(super) enum LandingAction {
    ChooseFolder,
    OpenLocation(String),
    Connect(crate::creds::SavedConnection),
    OpenGDrive,
    NewConnection,
    BuildIndex,
    RefreshIndex,
    ShowSyncJobs,
    ShowShare,
}

#[derive(Clone)]
pub(super) struct LandingTile {
    title: String,
    detail: String,
    meta: String,
    pub(super) action: Option<LandingAction>,
    meter: Option<(f32, String)>,
    warn: bool,
}

impl LandingTile {
    pub(super) fn action(
        title: impl Into<String>,
        detail: impl Into<String>,
        meta: impl Into<String>,
        action: LandingAction,
    ) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            meta: meta.into(),
            action: Some(action),
            meter: None,
            warn: false,
        }
    }

    pub(super) fn status(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            meta: String::new(),
            action: None,
            meter: None,
            warn: false,
        }
    }

    pub(super) fn meter(mut self, fraction: f32, label: impl Into<String>) -> Self {
        self.meter = Some((fraction.clamp(0.0, 1.0), label.into()));
        self
    }

    pub(super) fn warn(mut self, warn: bool) -> Self {
        self.warn = warn;
        self
    }
}

pub(super) fn ui_landing_section(
    ui: &mut egui::Ui,
    title: &str,
    default_open: bool,
    tiles: &[LandingTile],
    action: &mut Option<LandingAction>,
) {
    let header = egui::CollapsingHeader::new(
        RichText::new(format!("{} ({})", title, tiles.len()))
            .strong()
            .color(Color32::from_gray(180)),
    )
    .id_salt(("landing_section", title))
    .default_open(default_open)
    .show(ui, |ui| {
        ui.add_space(4.0);
        landing_tile_grid(ui, tiles, action);
    });
    ui.add_space(if header.fully_open() { 12.0 } else { 6.0 });
}

fn landing_tile_grid(ui: &mut egui::Ui, tiles: &[LandingTile], action: &mut Option<LandingAction>) {
    if tiles.is_empty() {
        ui.colored_label(Color32::from_gray(125), "Leer");
        return;
    }
    let gap = 8.0;
    let min_width = 210.0;
    let max_width = 330.0;
    let available = ui.available_width().max(min_width);
    let columns = ((available + gap) / (min_width + gap)).floor().max(1.0) as usize;
    let tile_width = ((available - gap * (columns.saturating_sub(1) as f32)) / columns as f32)
        .clamp(min_width.min(available), max_width);

    for row in tiles.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for tile in row {
                let response = landing_tile(ui, tile, tile_width);
                if response.clicked() {
                    if let Some(next) = tile.action.clone() {
                        *action = Some(next);
                    }
                }
            }
        });
        ui.add_space(gap);
    }
}

fn landing_tile(ui: &mut egui::Ui, tile: &LandingTile, width: f32) -> egui::Response {
    let height = if tile.meter.is_some() {
        88.0
    } else if tile.meta.is_empty() {
        58.0
    } else {
        72.0
    };
    let sense = if tile.action.is_some() {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);
    response.widget_info(|| landing_tile_widget_info(tile));

    if ui.is_rect_visible(rect) {
        paint_landing_tile(ui, rect, &response, tile);
    }

    let hover = [tile.detail.as_str(), tile.meta.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if hover.is_empty() {
        response
    } else {
        response.on_hover_text(hover)
    }
}

fn landing_tile_widget_info(tile: &LandingTile) -> egui::WidgetInfo {
    let typ = if tile.action.is_some() {
        egui::WidgetType::Button
    } else {
        egui::WidgetType::Label
    };
    let mut info = egui::WidgetInfo::labeled(typ, true, landing_tile_accessible_name(tile));
    if let Some((fraction, _)) = &tile.meter {
        info.value = Some(f64::from(*fraction));
    }
    info
}

fn landing_tile_accessible_name(tile: &LandingTile) -> String {
    let mut parts = vec![tile.title.as_str()];
    if !tile.detail.is_empty() {
        parts.push(tile.detail.as_str());
    }
    if !tile.meta.is_empty() {
        parts.push(tile.meta.as_str());
    }
    if let Some((_, label)) = &tile.meter {
        if !label.is_empty() {
            parts.push(label.as_str());
        }
    }
    let mut name = parts.join(". ");
    if tile.warn {
        name.push_str(". Warnung");
    }
    name
}

fn paint_landing_tile(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    tile: &LandingTile,
) {
    let visuals = ui.style().interact(response);
    let fill = if response.hovered() && tile.action.is_some() {
        visuals.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    let stroke_color = if tile.warn {
        Color32::from_rgb(220, 150, 80)
    } else {
        ui.visuals().widgets.inactive.bg_stroke.color
    };
    ui.painter().rect_filled(rect.shrink(0.5), 6.0, fill);
    ui.painter()
        .rect_stroke(rect.shrink(0.5), 6.0, egui::Stroke::new(1.0, stroke_color));

    let accent = if tile.warn {
        Color32::from_rgb(220, 150, 80)
    } else if tile.action.is_some() {
        ui.visuals().selection.bg_fill
    } else {
        Color32::from_gray(100)
    };
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(0.0, 8.0),
            egui::pos2(rect.left() + 3.0, rect.bottom() - 8.0),
        ),
        2.0,
        accent,
    );

    let x0 = rect.left() + 12.0;
    let x1 = rect.right() - 10.0;
    paint_landing_text(
        ui,
        egui::Rect::from_min_max(
            egui::pos2(x0, rect.top() + 8.0),
            egui::pos2(x1, rect.top() + 28.0),
        ),
        &tile.title,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    paint_landing_text(
        ui,
        egui::Rect::from_min_max(
            egui::pos2(x0, rect.top() + 30.0),
            egui::pos2(x1, rect.top() + 49.0),
        ),
        &tile.detail,
        egui::TextStyle::Small.resolve(ui.style()),
        Color32::from_gray(135),
    );
    if !tile.meta.is_empty() {
        let color = if tile.warn {
            Color32::from_rgb(230, 175, 95)
        } else {
            Color32::from_gray(150)
        };
        paint_landing_text(
            ui,
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.top() + 50.0),
                egui::pos2(x1, rect.top() + 68.0),
            ),
            &tile.meta,
            egui::TextStyle::Small.resolve(ui.style()),
            color,
        );
    }
    if let Some((fraction, label)) = &tile.meter {
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x0, rect.bottom() - 16.0),
            egui::pos2(x1, rect.bottom() - 10.0),
        );
        ui.painter()
            .rect_filled(bar_rect, 3.0, ui.visuals().widgets.inactive.bg_fill);
        let fill_rect = egui::Rect::from_min_max(
            bar_rect.left_top(),
            egui::pos2(
                bar_rect.left() + bar_rect.width() * *fraction,
                bar_rect.bottom(),
            ),
        );
        ui.painter().rect_filled(fill_rect, 3.0, accent);
        paint_landing_text(
            ui,
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.bottom() - 32.0),
                egui::pos2(x1, rect.bottom() - 17.0),
            ),
            label,
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::from_gray(135),
        );
    }
}

fn paint_landing_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    content: &str,
    font_id: egui::FontId,
    color: Color32,
) {
    if content.is_empty() {
        return;
    }
    use egui::text::{LayoutJob, TextWrapping};
    let mut job = LayoutJob::simple_singleline(content.to_string(), font_id, color);
    job.wrap = TextWrapping::truncate_at_width(rect.width().max(8.0));
    let galley = ui.fonts(|f| f.layout_job(job));
    ui.painter().galley(rect.left_top(), galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessible_name_includes_visible_state() {
        let tile = LandingTile::status("Disk", "42 GB frei")
            .meter(0.5, "50% belegt")
            .warn(true);
        let name = landing_tile_accessible_name(&tile);
        assert_eq!(name, "Disk. 42 GB frei. 50% belegt. Warnung");
        let info = landing_tile_widget_info(&tile);
        assert_eq!(info.typ, egui::WidgetType::Label);
        assert_eq!(info.value, Some(0.5));
    }

    #[test]
    fn actionable_tile_is_exposed_as_button() {
        let tile = LandingTile::action("Ordner", "Oeffnen", "Browse", LandingAction::ChooseFolder);
        let info = landing_tile_widget_info(&tile);
        assert_eq!(info.typ, egui::WidgetType::Button);
        assert_eq!(info.label.as_deref(), Some("Ordner. Oeffnen. Browse"));
    }
}
