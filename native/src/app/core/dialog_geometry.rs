//! Reachable, resizable dialog geometry. Storage contains only dialog-kind keys
//! and numeric dimensions; titles, paths and widget memory are never persisted.
use eframe::egui;

pub(super) struct DialogWindow<'open> {
    window: egui::Window<'open>,
    key: &'static str,
    id: egui::Id,
    default_size: egui::Vec2,
}

pub(super) fn window<'open>(
    ctx: &egui::Context,
    key: &'static str,
    title: impl Into<egui::WidgetText>,
) -> DialogWindow<'open> {
    let limit = super::theme::window_content_limit(ctx);
    DialogWindow {
        window: egui::Window::new(title)
            .id(egui::Id::new(key))
            .resizable(true)
            .min_size(limit.min(egui::vec2(220.0, 96.0)))
            .max_size(limit)
            .constrain_to(reachable_rect(ctx))
            .vscroll(true),
        key,
        id: egui::Id::new(key),
        default_size: egui::vec2(340.0, 420.0),
    }
}

impl<'open> DialogWindow<'open> {
    pub(super) fn show<R>(
        mut self,
        ctx: &egui::Context,
        contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<egui::InnerResponse<Option<R>>> {
        let limit = super::theme::window_content_limit(ctx);
        let saved = super::dialog_geometry_store::load(self.key);
        let initial = saved.map(egui::Vec2::from).unwrap_or(self.default_size).min(limit);
        self.window = self.window.default_size(initial);
        let correction_id = egui::Id::new(("picker_initial_correction", self.key));
        let fresh_picker = self.key.starts_with("picker.") && saved.is_none()
            && ctx.memory(|memory| memory.area_rect(self.id).is_none());
        if let Some(height) = ctx.data_mut(|data| data.remove_temp::<f32>(correction_id)) {
            if !ctx.input(|input| input.pointer.any_down()) {
                // One settling frame uses measured content chrome. Afterwards
                // normal resizing resumes; existing user sizes never enter here.
                let height = height.clamp(1.0, limit.y);
                self.window = self.window.min_height(height).max_height(height);
            }
        }
        let result = self.window.show(ctx, |ui| {
            let popup_height = (ctx.screen_rect().height() - ui.spacing().menu_margin.sum().y)
                .max(ui.spacing().interact_size.y);
            ui.spacing_mut().combo_height = popup_height;
            if super::dialog_geometry_store::failed() {
                ui.colored_label(super::theme::warning(ui),
                    "Die Dialoggröße konnte nicht dauerhaft gespeichert werden.");
            }
            contents(ui)
        });
        {
            if let Some(response) = &result {
                if response.inner.is_some() {
                    let style = ctx.style();
                    let frame = egui::Frame::window(&style);
                    let title = ctx.fonts(|fonts| {
                        fonts.row_height(&egui::TextStyle::Heading.resolve(&style))
                    }) + frame.inner_margin.sum().y;
                    let chrome = frame.inner_margin.sum() + frame.outer_margin.sum()
                        + egui::vec2(0.0, title);
                    let size = (response.response.rect.size() - chrome)
                        .max(egui::Vec2::splat(1.0)).min(limit);
                    if fresh_picker {
                        if let Some(list) = ctx.data(|data| data.get_temp::<f32>(
                            egui::Id::new(("picker_list_height", self.key)))) {
                            let content_chrome = size.y - list;
                            let target = 160.0_f32.min((limit.y - content_chrome).max(1.0));
                            ctx.data_mut(|data| data.insert_temp(correction_id,
                                (content_chrome + target).min(limit.y)));
                            ctx.request_repaint();
                        }
                    }
                    let state_id = egui::Id::new(("dialog_geometry_observed", self.key));
                    let previous = ctx.data(|data| data.get_temp::<([f32; 2], bool)>(state_id));
                    let down = ctx.input(|input| input.pointer.any_down());
                    let changed = previous.is_some_and(|(old, _)| {
                        (old[0] - size.x).abs() >= 0.5 || (old[1] - size.y).abs() >= 0.5
                    });
                    let pending = previous.is_some_and(|(_, pending)| pending) || (down && changed);
                    if pending && !down {
                        super::dialog_geometry_store::save(self.key, [size.x, size.y]);
                    }
                    ctx.data_mut(|data| data.insert_temp(state_id,
                        ([size.x, size.y], pending && down)));
                }
            }
        }
        result
    }

    pub(super) fn default_size(mut self, size: impl Into<egui::Vec2>) -> Self {
        self.default_size = size.into(); self
    }
    pub(super) fn default_width(mut self, width: f32) -> Self {
        self.default_size.x = width; self
    }
    pub(super) fn open(mut self, open: &'open mut bool) -> Self {
        self.window = self.window.open(open); self
    }
    pub(super) fn id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self.window = self.window.id(id); self
    }
    pub(super) fn collapsible(mut self, value: bool) -> Self {
        self.window = self.window.collapsible(value); self
    }
    pub(super) fn resizable(mut self, value: bool) -> Self {
        self.window = self.window.resizable(value); self
    }
    pub(super) fn vscroll(mut self, value: bool) -> Self {
        self.window = self.window.vscroll(value); self
    }
    pub(super) fn max_size(mut self, value: impl Into<egui::Vec2>) -> Self {
        self.window = self.window.max_size(value); self
    }
    pub(super) fn max_width(mut self, value: f32) -> Self {
        self.window = self.window.max_width(value); self
    }
    pub(super) fn max_height(mut self, value: f32) -> Self {
        self.window = self.window.max_height(value); self
    }
    pub(super) fn min_width(mut self, value: f32) -> Self {
        self.window = self.window.min_width(value); self
    }
    pub(super) fn constrain_to(mut self, value: egui::Rect) -> Self {
        let value = if value.width() > 0.0 && value.height() > 0.0 {
            value
        } else { egui::Rect::from_min_size(value.center(), egui::Vec2::splat(1.0)) };
        self.window = self.window.constrain_to(value); self
    }
    pub(super) fn constrain(mut self, value: bool) -> Self {
        self.window = self.window.constrain(value); self
    }
    pub(super) fn anchor(mut self, align: egui::Align2, offset: impl Into<egui::Vec2>) -> Self {
        self.window = self.window.anchor(align, offset); self
    }
    pub(super) fn order(mut self, order: egui::Order) -> Self {
        self.window = self.window.order(order); self
    }
}

/// Height left in the actual viewport, including nested scroll areas.
pub(super) fn remaining_height(ui: &egui::Ui, action_rows: f32) -> f32 {
    let visible = (ui.clip_rect().max.y - ui.cursor().min.y).max(0.0);
    let reserve = action_rows * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
    (ui.available_height().min(visible) - reserve).max(1.0)
}

/// The previous horizontal-row calculation had an 80-point list floor.
/// The initial target doubles that usable list, not the old outer window.
/// Header/footer dimensions follow current font metrics and control spacing.
pub(super) fn picker_initial_size(ctx: &egui::Context) -> egui::Vec2 {
    let style = ctx.style();
    let row = style.spacing.interact_size.y;
    let label = ctx.fonts(|fonts| fonts.row_height(&egui::TextStyle::Body.resolve(&style)));
    let spacing = style.spacing.item_spacing.y;
    let header = 2.0 * row + label + 6.0 + 4.0 * spacing;
    let footer = row + label + 6.0 + 4.0 * spacing;
    egui::vec2(760.0, 160.0 + header + footer)
        .min(super::theme::window_content_limit(ctx))
}

pub(super) fn picker_footer_height(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y + ui.text_style_height(&egui::TextStyle::Body)
        + 6.0 + 4.0 * ui.spacing().item_spacing.y
}

fn reachable_rect(ctx: &egui::Context) -> egui::Rect {
    let screen = ctx.screen_rect();
    screen.shrink2(egui::vec2((screen.width() / 4.0).min(16.0),
        (screen.height() / 4.0).min(16.0)))
}
