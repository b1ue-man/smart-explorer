use super::analytics_measure::{MeasureView, MEASURE_HINT};
use super::analytics_view::{is_system_files, SYSTEM_FILES_HINT};
use super::prelude::*;
use crate::app::theme;

/// A virtualized, keyboard- and screen-reader-friendly representation of the
/// current treemap level. The painted treemap remains the fast spatial view;
/// this list exposes every direct child without creating an accessibility node
/// for every recursively painted cell on every frame. Its columns „Größe“ and
/// „Belegt“ show both values; the rows follow „Sortieren/Kacheln nach“.
pub(super) fn treemap_accessible_list(
    ui: &mut egui::Ui,
    node: Option<&crate::analytics::SizeNode>,
    base: &str,
    measure: MeasureView,
    drill_path: &mut Option<String>,
    reveal: &mut Option<String>,
) {
    let Some(node) = node else {
        return;
    };
    let children = &node.children;
    egui::CollapsingHeader::new(format!("Barrierefreie Elementliste ({})", children.len()))
        .id_salt(("analytics_element_list", base))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "Virtuelle Liste der aktuellen Ebene, sortiert nach {}; \
                     Ordner öffnen die nächste Ebene.",
                    measure.effective().label()
                ))
                .small()
                .color(theme::muted(ui)),
            );
            let row_height = ui.spacing().interact_size.y.max(24.0);
            let columns = ui.available_width();
            let value_width = 96.0_f32.min(columns / 4.0);
            let name_width = (columns - 2.0 * value_width).max(0.0);
            ui.horizontal(|ui| {
                ui.add_sized([name_width, row_height], egui::Label::new("Name"));
                for title in ["Größe", "Belegt"] {
                    ui.add_sized([value_width, row_height], egui::Label::new(title))
                        .on_hover_text(MEASURE_HINT);
                }
            });
            ui.horizontal(|ui| {
                ui.add_sized(
                    [name_width, row_height],
                    egui::Label::new(RichText::new("Gesamt").strong()),
                );
                let total = [
                    format_bytes(node.size),
                    measure.allocated_text(node.allocated),
                ];
                for value in total {
                    ui.add_sized(
                        [value_width, row_height],
                        egui::Label::new(RichText::new(value).strong()),
                    );
                }
            });
            egui::ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .id_salt(("analytics_element_scroll", base))
                .max_height(super::dialog_geometry::remaining_height(ui, 2.0))
                .auto_shrink([false, false])
                .show_rows(ui, row_height, children.len(), |ui, rows| {
                    for index in rows {
                        // Sorted once by the measure (`MeasureView::sort_tree`).
                        let child = &children[index];
                        let path = format!("{}/{}", base.trim_end_matches('/'), child.name);
                        let kind = if child.is_dir { "Ordner" } else { "Datei" };
                        let allocated = measure.allocated_text(child.allocated);
                        let widths = (name_width, value_width, row_height);
                        let response = element_row(ui, kind, child, &allocated, widths);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!(
                                    "{kind}: {}. Größe: {}. Belegt: {allocated}. Pfad: {path}",
                                    child.name,
                                    format_bytes(child.size)
                                ),
                            )
                        });
                        let system = is_system_files(&child.name, child.is_dir);
                        let response = if system {
                            response.on_hover_text(SYSTEM_FILES_HINT)
                        } else {
                            response
                        };
                        if response.clicked() {
                            if child.is_dir {
                                *drill_path = Some(path);
                            } else if !system {
                                *reveal = Some(path);
                            }
                        }
                    }
                });
        });
}

/// One row: the name as the button, then „Größe“ and „Belegt“.
fn element_row(
    ui: &mut egui::Ui,
    kind: &str,
    child: &crate::analytics::SizeNode,
    allocated: &str,
    (name_width, value_width, row_height): (f32, f32, f32),
) -> egui::Response {
    ui.horizontal(|ui| {
        let name = egui::Button::new(format!("{kind} {}", child.name)).truncate();
        let response = ui.add_sized([name_width, row_height], name);
        let size = egui::Label::new(format_bytes(child.size));
        ui.add_sized([value_width, row_height], size);
        ui.add_sized([value_width, row_height], egui::Label::new(allocated));
        response
    })
    .inner
}
