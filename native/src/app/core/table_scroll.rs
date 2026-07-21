use eframe::egui;

const FILE_TABLE_SCROLL_ID: &str = "file_table_horizontal";

pub(super) fn show_horizontal_file_table<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    egui::ScrollArea::horizontal()
        .id_salt(FILE_TABLE_SCROLL_ID)
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .show(ui, add_contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct ScrollMetrics {
        content_width: f32,
        viewport_width: f32,
        offset_x: f32,
    }

    fn render_file_table(context: &egui::Context, input: egui::RawInput) -> ScrollMetrics {
        let mut metrics = None;
        let _ = context.run(input, |context| {
            egui::SidePanel::right("remote_details_test_panel")
                .exact_width(420.0)
                .show(context, |ui| {
                    ui.label("Remote-Details");
                });
            egui::CentralPanel::default().show(context, |ui| {
                let output = show_horizontal_file_table(ui, |ui| {
                    ui.allocate_exact_size(egui::vec2(1_200.0, 80.0), egui::Sense::hover());
                });
                metrics = Some(ScrollMetrics {
                    content_width: output.content_size.x,
                    viewport_width: output.inner_rect.width(),
                    offset_x: output.state.offset.x,
                });
            });
        });
        metrics.expect("the central file-table panel must be rendered")
    }

    fn narrow_window_input(time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(760.0, 260.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn remote_drive_task_wide_file_table_scrolls_beside_details_panel() {
        let context = egui::Context::default();
        let pointer = egui::pos2(150.0, 100.0);
        let initial = render_file_table(
            &context,
            narrow_window_input(0.0, vec![egui::Event::PointerMoved(pointer)]),
        );

        assert!(
            initial.content_width > initial.viewport_width,
            "wide columns must overflow the clipped table viewport"
        );
        assert!(
            initial.viewport_width < 400.0,
            "the details panel must leave a meaningfully narrow file-table viewport"
        );

        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let scrolled = render_file_table(
            &context,
            narrow_window_input(
                1.0 / 60.0,
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -80.0),
                        modifiers: shift,
                    },
                ],
            ),
        );

        assert!(
            scrolled.offset_x > initial.offset_x,
            "Shift+wheel over the clipped file table must change its horizontal offset"
        );
        assert!(
            scrolled.offset_x <= scrolled.content_width - scrolled.viewport_width,
            "the horizontal offset must remain inside the scrollable content"
        );
    }
}
