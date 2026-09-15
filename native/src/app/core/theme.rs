//! Shared presentation tokens for every window, including custom-painted UI.
use eframe::egui::{self, Color32, FontId, Margin, Rounding, Stroke, TextStyle};

pub(super) const TABLE_HEADER_HEIGHT: f32 = 34.0;

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
                surface: rgb(24, 30, 40), panel: rgb(30, 37, 49),
                subtle: rgb(36, 45, 59), text: rgb(236, 241, 248),
                muted: rgb(177, 190, 208), border: rgb(65, 78, 96),
                control_border: rgb(120, 137, 160), accent: rgb(129, 181, 255),
                selection: rgb(43, 70, 110), selected_text: rgb(242, 247, 255),
                success: rgb(125, 216, 166), warning: rgb(255, 203, 120),
                danger: rgb(255, 154, 158),
            }
        } else {
            Self {
                surface: rgb(255, 255, 255), panel: rgb(245, 247, 250),
                subtle: rgb(235, 240, 247), text: rgb(25, 36, 53),
                muted: rgb(74, 88, 108), border: rgb(199, 208, 221),
                control_border: rgb(108, 123, 144), accent: rgb(25, 83, 158),
                selection: rgb(214, 230, 252), selected_text: rgb(18, 53, 104),
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
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Heading, FontId::proportional(22.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]);
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.window_margin = Margin::same(16.0);
    style.spacing.menu_margin = Margin::same(10.0);
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
    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.5, p.selected_text);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    for widget in [&mut v.widgets.noninteractive, &mut v.widgets.inactive,
        &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        widget.bg_fill = p.subtle;
        widget.weak_bg_fill = p.subtle;
        widget.bg_stroke = Stroke::new(1.0, p.control_border);
        widget.fg_stroke = Stroke::new(1.5, p.text);
        widget.rounding = Rounding::same(6.0);
        widget.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    // egui derives hint/weak text by tinting toward this color. A muted target
    // keeps hints readable in 0.29, which has no separate weak-text override.
    v.widgets.noninteractive.weak_bg_fill = p.muted;
    v.widgets.hovered.bg_fill = p.selection;
    v.widgets.hovered.weak_bg_fill = p.selection;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.bg_fill = p.selection;
    v.widgets.active.weak_bg_fill = p.selection;
    v.widgets.active.bg_stroke = Stroke::new(2.0, p.accent);
    v.widgets.open.bg_fill = p.selection;
    v.widgets.open.weak_bg_fill = p.selection;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    style
}

pub(super) fn install(ctx: &egui::Context, mode: super::ui_preferences::ColorMode) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
    ctx.set_theme(mode.preference());
}

pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(title).strong().color(muted(ui)));
    ui.add_space(2.0);
}
