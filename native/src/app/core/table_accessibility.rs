use super::prelude::*;

pub(super) struct TableRowSemantics<'a> {
    name: &'a str,
    path: &'a str,
    kind: &'static str,
    selected: bool,
}

impl<'a> TableRowSemantics<'a> {
    pub(super) fn new(name: &'a str, path: &'a str, is_dir: bool, selected: bool) -> Self {
        Self {
            name,
            path,
            kind: if is_dir { "Ordner" } else { "Datei" },
            selected,
        }
    }

    pub(super) fn annotate_cell(&self, response: &egui::Response, column: &str, value: &str) {
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                self.selected,
                self.cell_label(column, value),
            )
        });
    }

    fn cell_label(&self, column: &str, value: &str) -> String {
        let value = if value.is_empty() { "leer" } else { value };
        format!(
            "{column}: {value}. {}, {}. Pfad: {}",
            self.kind, self.name, self.path
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_label_names_column_value_and_row() {
        let row = TableRowSemantics::new("bericht.txt", "/tmp/bericht.txt", false, true);
        assert_eq!(
            row.cell_label("Größe", "12 KB"),
            "Größe: 12 KB. Datei, bericht.txt. Pfad: /tmp/bericht.txt"
        );
    }

    #[test]
    fn empty_cell_value_is_explicit() {
        let row = TableRowSemantics::new("Ordner", "/tmp/Ordner", true, false);
        assert!(row.cell_label("Typ", "").starts_with("Typ: leer. Ordner"));
    }
}
