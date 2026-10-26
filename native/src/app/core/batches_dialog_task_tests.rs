use super::*;

fn input(size: egui::Vec2) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default() }
}

#[test]
fn batches_task_dialog_real_egui_rect_stays_reachable_after_viewport_shrink() {
    let ctx = egui::Context::default();
    for size in [egui::vec2(1200.0, 900.0), egui::vec2(360.0, 300.0)] {
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..4 {
            let _ = ctx.run(input(size), |ctx| {
                rect = window(ctx, "batches.fixture.dialog", "Dialog")
                    .default_size([900.0, 700.0]).collapsible(false)
                    .show(ctx, |ui| {
                        for row in 0..40 { ui.label(format!("Zeile {row}")); }
                    }).unwrap().response.rect;
            });
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert!(screen.expand(1.0).contains_rect(rect), "screen={screen:?}, dialog={rect:?}");
        assert!(rect.width() > 0.0 && rect.height() > 0.0);
    }
}

#[test]
fn batches_task_picker_height_reserves_actual_chrome_and_action_rows() {
    for size in [egui::vec2(1280.0, 1000.0), egui::vec2(360.0, 300.0)] {
        let ctx = egui::Context::default();
        let _ = ctx.run(input(size), |ctx| {
            let style = ctx.style();
            let label = ctx.fonts(|fonts| fonts.row_height(&egui::TextStyle::Body.resolve(&style)));
            let row = style.spacing.interact_size.y;
            let spacing = style.spacing.item_spacing.y;
            let content_chrome = 3.0 * row + 2.0 * label + 12.0 + 8.0 * spacing;
            let limit = super::super::theme::window_content_limit(ctx);
            let initial = picker_initial_size(ctx);
            assert!((initial.y - (160.0 + content_chrome).min(limit.y)).abs() < 0.01);
            assert!(initial.x <= limit.x && initial.y <= limit.y);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Ordner wählen");
                let available = ui.available_height().min(
                    (ui.clip_rect().max.y - ui.cursor().min.y).max(0.0));
                let actions = 2.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
                assert!((remaining_height(ui, 2.0) - (available - actions).max(1.0)).abs() < 0.01);
                assert!(picker_footer_height(ui) >= ui.spacing().interact_size.y + label);
            });
        });
    }
}
