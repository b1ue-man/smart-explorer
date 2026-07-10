use super::prelude::*;

/// A virtualized, keyboard- and screen-reader-friendly representation of the
/// current treemap level. The painted treemap remains the fast spatial view;
/// this list exposes every direct child without creating an accessibility node
/// for every recursively painted cell on every frame.
pub(super) fn treemap_accessible_list(
    ui: &mut egui::Ui,
    node: Option<&crate::analytics::SizeNode>,
    base: &str,
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
                RichText::new(
                    "Virtuelle Liste der aktuellen Ebene; Ordner öffnen die nächste Ebene.",
                )
                .small()
                .color(Color32::from_gray(150)),
            );
            let row_height = ui.spacing().interact_size.y.max(24.0);
            egui::ScrollArea::vertical()
                .id_salt(("analytics_element_scroll", base))
                .max_height(180.0)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, children.len(), |ui, rows| {
                    for index in rows {
                        let child = &children[index];
                        let path = format!("{}/{}", base.trim_end_matches('/'), child.name);
                        let kind = if child.is_dir { "Ordner" } else { "Datei" };
                        let visible = format!(
                            "{} {} — {}",
                            if child.is_dir { "📁" } else { "📄" },
                            child.name,
                            format_bytes(child.size)
                        );
                        let response = ui.add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::new(visible),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!(
                                    "{kind}: {}. Größe: {}. Pfad: {path}",
                                    child.name,
                                    format_bytes(child.size)
                                ),
                            )
                        });
                        if response.clicked() {
                            if child.is_dir {
                                *drill_path = Some(path);
                            } else {
                                *reveal = Some(path);
                            }
                        }
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn child_path_does_not_double_the_root_separator() {
        let base = "/";
        let name = "tmp";
        assert_eq!(format!("{}/{}", base.trim_end_matches('/'), name), "/tmp");
    }
}
