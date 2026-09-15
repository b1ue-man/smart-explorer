//! Shared presentation tokens for every window, including custom-painted UI.
use eframe::egui::{self, Color32, FontId, Margin, Rounding, Stroke, TextStyle};

pub(super) const TABLE_HEADER_HEIGHT: f32 = 24.0;

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) surface: Color32,
    pub(super) panel: Color32,
    pub(super) subtle: Color32,
    pub(super) text: Color32,
    pub(super) muted: Color32,
    pub(super) border: Color32,
    pub(super) control_border: Color32,
    pub(super) accent: Color32,
    pub(super) selection: Color32,
    pub(super) selected_text: Color32,
    pub(super) success: Color32,
    pub(super) warning: Color32,
    pub(super) danger: Color32,
}

impl Palette {
    pub(super) fn new(dark: bool) -> Self {
        let rgb = Color32::from_rgb;
        if dark {
            Self {
                surface: rgb(27, 27, 27), panel: rgb(32, 32, 32),
                subtle: rgb(40, 40, 40), text: rgb(225, 225, 225),
                muted: rgb(175, 175, 175), border: rgb(65, 65, 65),
                control_border: rgb(125, 125, 125), accent: rgb(125, 180, 235),
                selection: rgb(40, 65, 90), selected_text: rgb(240, 245, 250),
                success: rgb(125, 216, 166), warning: rgb(255, 203, 120),
                danger: rgb(255, 154, 158),
            }
        } else {
            Self {
                surface: rgb(255, 255, 255), panel: rgb(245, 245, 245),
                subtle: rgb(237, 237, 237), text: rgb(30, 30, 30),
                muted: rgb(85, 85, 85), border: rgb(195, 195, 195),
                control_border: rgb(115, 115, 115), accent: rgb(25, 80, 145),
                selection: rgb(211, 228, 245), selected_text: rgb(20, 45, 75),
                success: rgb(25, 106, 63), warning: rgb(135, 76, 10),
                danger: rgb(173, 38, 50),
            }
        }
    }
}

pub(super) fn palette(ui: &egui::Ui) -> Palette { Palette::new(ui.visuals().dark_mode) }
pub(super) fn muted(ui: &egui::Ui) -> Color32 { palette(ui).muted }
pub(super) fn accent(ui: &egui::Ui) -> Color32 { palette(ui).accent }
pub(super) fn success(ui: &egui::Ui) -> Color32 { palette(ui).success }
pub(super) fn warning(ui: &egui::Ui) -> Color32 { palette(ui).warning }
pub(super) fn danger(ui: &egui::Ui) -> Color32 { palette(ui).danger }

pub(super) fn style(theme: egui::Theme) -> egui::Style {
    let mut style = theme.default_style();
    let p = Palette::new(theme == egui::Theme::Dark);
    style.text_styles.extend([
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Heading, FontId::proportional(18.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]);
    style.spacing.item_spacing = egui::vec2(6.0, 3.0);
    style.spacing.button_padding = egui::vec2(5.0, 2.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.window_margin = Margin::same(8.0);
    style.spacing.menu_margin = Margin::same(6.0);
    style.spacing.indent = 16.0;
    style.spacing.combo_height = 280.0;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 10.0;
    style.animation_time = 0.12;
    let v = &mut style.visuals;
    v.panel_fill = p.panel;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.subtle;
    v.code_bg_color = p.subtle;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_rounding = Rounding::same(2.0);
    v.menu_rounding = Rounding::same(2.0);
    v.window_highlight_topmost = false;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.5_f32, p.selected_text);
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.accent);
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive,
        &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        widget.fg_stroke = Stroke::new(1.0_f32, p.text);
        widget.rounding = Rounding::same(2.0);
        widget.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    // egui derives hint/weak text by tinting toward this color. A muted target
    // keeps hints readable in 0.29, which has no separate weak-text override.
    v.widgets.noninteractive.weak_bg_fill = p.muted;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.control_border);
    v.widgets.active.bg_stroke = Stroke::new(2.0_f32, p.accent);
    style
}

pub(super) fn install(ctx: &egui::Context, mode: super::ui_preferences::ColorMode) {
    // Ubuntu-Light lacks navigation/math symbols. Hack is already bundled;
    // add it only as a fallback, preserving proportional text and emoji.
    let mut fonts = egui::FontDefinitions::default();
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.push("Hack".to_string());
    }
    ctx.set_fonts(fonts);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
    ctx.set_theme(mode.preference());
}

pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).small().strong().color(muted(ui)));
    ui.add_space(2.0);
}

/// egui 0.29's Resize limit covers content; reserve frame and title chrome too.
pub(super) fn window_content_limit(ctx: &egui::Context) -> egui::Vec2 {
    let style = ctx.style();
    let margin = style.spacing.window_margin.sum();
    let title = ctx.fonts(|fonts| fonts.row_height(&TextStyle::Heading.resolve(&style))) + margin.y;
    (ctx.screen_rect().size() - egui::vec2(32.0, 32.0) - margin - egui::vec2(0.0, title))
        .max(egui::vec2(1.0, 1.0))
}
