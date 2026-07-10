use super::prelude::*;

pub(in crate::app) fn ui_section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| add(ui));
}

pub(in crate::app) fn result_count_label(title: &str, shown: usize, total: u64) -> String {
    if total > shown as u64 {
        format!("{title} (Top {shown} von {total})")
    } else {
        format!("{title} ({shown})")
    }
}

pub(in crate::app) fn ui_items(
    ui: &mut egui::Ui,
    title: &str,
    items: &[crate::analytics::ReclaimItem],
    total: u64,
    selected: &mut HashSet<String>,
    reveal: &mut Option<String>,
) {
    egui::CollapsingHeader::new(result_count_label(title, items.len(), total))
        .default_open(false)
        .show(ui, |ui| {
            if items.is_empty() {
                ui_empty(ui);
            }
            for item in items {
                ui_item(ui, item, selected, reveal, false);
            }
        });
}

pub(in crate::app) fn ui_item(
    ui: &mut egui::Ui,
    item: &crate::analytics::ReclaimItem,
    selected: &mut HashSet<String>,
    reveal: &mut Option<String>,
    first_duplicate: bool,
) {
    ui.horizontal(|ui| {
        let mut on = selected.contains(&item.path);
        let checkbox = ui.add(egui::Checkbox::without_text(&mut on));
        checkbox.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                on,
                reclaim_selection_label(item),
            )
        });
        if checkbox.changed() {
            if on {
                selected.insert(item.path.clone());
            } else {
                selected.remove(&item.path);
            }
        }
        ui.label(RichText::new(format_bytes(item.size)).monospace());
        if first_duplicate {
            ui.label(
                RichText::new("behalten")
                    .small()
                    .color(Color32::from_gray(140)),
            );
        }
        let date = if item.mtime_ms > 0 {
            format_date(item.mtime_ms)
        } else {
            "-".to_string()
        };
        ui.label(RichText::new(date).small().color(Color32::from_gray(150)));
        ui.add(egui::Label::new(&item.name).truncate())
            .on_hover_text(&item.path);
        let reason = if item.reason.is_empty() {
            item.confidence.label().to_string()
        } else {
            format!("{} · {}", item.reason, item.confidence.label())
        };
        ui.label(RichText::new(reason).small().color(Color32::from_gray(150)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Anzeigen").clicked() {
                *reveal = Some(item.path.clone());
            }
        });
    });
}

fn reclaim_selection_label(item: &crate::analytics::ReclaimItem) -> String {
    let reason = if item.reason.is_empty() {
        item.confidence.label().to_string()
    } else {
        format!("{}; {}", item.reason, item.confidence.label())
    };
    format!(
        "Auswählen: {}, {}, {}. Pfad: {}",
        item.name,
        format_bytes(item.size),
        reason,
        item.path
    )
}

pub(in crate::app) fn ui_empty(ui: &mut egui::Ui) {
    ui.colored_label(Color32::from_gray(140), "(keine)");
}

pub(in crate::app) fn select_items(
    selected: &mut HashSet<String>,
    items: &[crate::analytics::ReclaimItem],
) {
    for item in items {
        if item.confidence.quick_selectable() {
            selected.insert(item.path.clone());
        }
    }
}

pub(in crate::app) fn selected_bytes(
    report: &crate::analytics::ReclaimReport,
    selected: &HashSet<String>,
) -> u64 {
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for item in report
        .large_files
        .iter()
        .chain(report.stale_files.iter())
        .chain(report.empty_files.iter())
        .chain(report.empty_dirs.iter())
        .chain(report.cleanup.iter())
        .chain(report.duplicate_groups.iter().flat_map(|g| g.items.iter()))
    {
        if selected.contains(&item.path) && seen.insert(item.path.as_str()) {
            total = total.saturating_add(item.size);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::result_count_label;

    #[test]
    fn count_label_discloses_truncation() {
        assert_eq!(
            result_count_label("Dateien", 200, 325),
            "Dateien (Top 200 von 325)"
        );
        assert_eq!(result_count_label("Dateien", 12, 12), "Dateien (12)");
    }
}
