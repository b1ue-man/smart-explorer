use super::{analytics_paint, navigation_path, theme, ui_preferences::*};
use eframe::egui::{self, Color32};

fn contrast(a: Color32, b: Color32) -> f64 {
    let luminance = |color: Color32| {
        let channels = [color.r(), color.g(), color.b()].map(|channel| {
            let c = f64::from(channel) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn gui_design_task_text_control_focus_and_chart_contrast() {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let style = theme::style(theme);
        let p = theme::Palette::new(style.visuals.dark_mode);
        for background in [p.surface, p.panel, p.subtle] {
            for foreground in [p.text, p.muted, p.accent, p.warning, p.success, p.danger,
                style.visuals.weak_text_color()] {
                let ratio = contrast(foreground, background);
                assert!(ratio >= 4.5, "{theme:?}: {foreground:?} on {background:?} = {ratio}");
            }
            for foreground in [p.control_border, p.accent] {
                assert!(contrast(foreground, background) >= 3.0);
            }
        }
        assert!(contrast(p.selected_text, p.selection) >= 4.5);
        for widget in [style.visuals.widgets.active, style.visuals.widgets.hovered,
            style.visuals.widgets.open] {
            assert!(contrast(widget.fg_stroke.color, widget.bg_fill) >= 4.5);
        }
        assert!(style.visuals.widgets.active.bg_stroke.width >= 2.0);
        assert!(style.text_styles[&egui::TextStyle::Small].size >= 12.0);
        for color in super::treemap::TM_PALETTE {
            for background in [color, color.gamma_multiply(0.7),
                egui::ecolor::tint_color_towards(color, p.surface)] {
                assert!(contrast(analytics_paint::label_color(background), background) >= 4.5);
            }
        }
    }
}

#[test]
fn gui_design_task_preferences_preserve_legacy_panels_and_round_trip() {
    let old = UiState::parse("show_filters=true\nshow_summary=1\nfuture=value\n");
    assert!(old.show_filters && old.show_summary);
    assert_eq!(old.appearance.mode, ColorMode::System);
    assert!(!old.appearance.detailed_columns);
    for mode in [ColorMode::Light, ColorMode::Dark, ColorMode::System] {
        let preferences = UiState {
            show_filters: false, show_summary: true,
            appearance: Appearance { mode, compact: true, detailed_columns: true },
        };
        assert_eq!(UiState::parse(&preferences.encode()), preferences);
    }
    assert_eq!(UiState::parse("color_mode=unknown").appearance.mode, ColorMode::System);
    assert!(!UiState::default().show_filters);
    assert!(Appearance::default().row_height() > Appearance { compact: true, ..Default::default() }.row_height());
}

#[test]
fn gui_design_task_theme_switch_keeps_custom_styles() {
    let ctx = egui::Context::default();
    theme::install(&ctx, ColorMode::System);
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let _ = ctx.run(egui::RawInput { system_theme: Some(system), ..Default::default() }, |_| {});
        assert_eq!(ctx.theme(), system);
        assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 14.0);
    }
    ctx.set_theme(egui::Theme::Light);
    let _ = ctx.run(egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() }, |_| {});
    assert_eq!(ctx.theme(), egui::Theme::Light);
}

#[test]
fn gui_design_task_breadcrumbs_keep_absolute_roots() {
    for (source, targets) in [
        ("/", vec!["/"]),
        ("/home/team", vec!["/", "/home/", "/home/team/"]),
        ("C:\\Work\\Notes", vec!["C:/", "C:/Work/", "C:/Work/Notes/"]),
        ("\\\\server\\share\\folder", vec!["//server/", "//server/share/", "//server/share/folder/"]),
    ] {
        let actual: Vec<_> = navigation_path::breadcrumbs(source).into_iter().map(|crumb| crumb.path).collect();
        assert_eq!(actual, targets, "{source}");
    }
}
