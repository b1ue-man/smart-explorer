//! The tabs „Größte Dateien“ and „Nach Dateityp“ of the storage analysis:
//! the shown folder with all its subfolders, both values per row, ordered by
//! „Sortieren/Kacheln nach“ (`crate::analytics_overview`).
use super::analytics_measure::{MeasureView, SizeMeasure, MEASURE_HINT};
use super::analytics_view::{is_system_files, AnalyticsTab, OVERVIEW_TOP, SYSTEM_FILES_HINT};
use super::prelude::*;
use super::*;
use crate::analytics_overview::{aggregated_note, Overview};
use crate::app::theme;

impl App {
    /// Builds the overview of the shown folder once a list tab is shown.
    pub(in crate::app) fn ensure_analytics_overview(&mut self) {
        let view = &self.analytics_view;
        if view.tab == AnalyticsTab::Treemap || view.overview.is_some() {
            return;
        }
        let by_allocated = view.measure.effective() == SizeMeasure::Allocated;
        let overview = self
            .analytics_focus_node()
            .map(|node| crate::analytics_overview::overview(node, by_allocated, OVERVIEW_TOP));
        self.analytics_view.overview = overview;
    }
}

/// The tab row; `Some` when the person picked another tab.
pub(super) fn tabs_ui(ui: &mut egui::Ui, shown: AnalyticsTab) -> Option<AnalyticsTab> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        for tab in AnalyticsTab::ALL {
            if ui.selectable_label(shown == tab, tab.label()).clicked() && tab != shown {
                picked = Some(tab);
            }
        }
    });
    picked
}

/// One list tab; a click on a file reveals it (`base` + its relative path).
pub(super) fn overview_ui(
    ui: &mut egui::Ui,
    tab: AnalyticsTab,
    overview: Option<&Overview>,
    measure: MeasureView,
    base: &str,
    reveal: &mut Option<String>,
) {
    let Some(overview) = overview else {
        ui.colored_label(theme::muted(ui), "Noch keine Scan-Daten vorhanden.");
        return;
    };
    ui.label(
        RichText::new(format!(
            "Ganzer angezeigter Ordner mit Unterordnern, sortiert nach {}.",
            measure.effective().label()
        ))
        .small()
        .color(theme::muted(ui)),
    );
    if let Some(note) = aggregated_note(&overview.aggregated) {
        ui.colored_label(theme::muted(ui), note);
    }
    egui::ScrollArea::vertical()
        .id_salt(("analytics_overview", base, tab.label()))
        .min_scrolled_height(0.0)
        .max_height(super::dialog_geometry::remaining_height(ui, 0.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new(("analytics_overview_grid", tab.label()))
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| match tab {
                    AnalyticsTab::Largest => largest_rows(ui, overview, measure, base, reveal),
                    AnalyticsTab::Types => type_rows(ui, overview, measure),
                    AnalyticsTab::Treemap => {}
                });
        });
}

fn header(ui: &mut egui::Ui, first: &str, last: &str) {
    ui.label(RichText::new(first).strong());
    ui.label(RichText::new("Größe").strong())
        .on_hover_text(MEASURE_HINT);
    ui.label(RichText::new("Belegt").strong())
        .on_hover_text(MEASURE_HINT);
    ui.label(RichText::new(last).strong());
    ui.end_row();
}

fn largest_rows(
    ui: &mut egui::Ui,
    overview: &Overview,
    measure: MeasureView,
    base: &str,
    reveal: &mut Option<String>,
) {
    header(ui, "Nr.", "Datei");
    for (index, file) in overview.largest.iter().enumerate() {
        ui.label((index + 1).to_string());
        ui.label(format_bytes(file.size));
        ui.label(measure.allocated_text(file.allocated));
        let path = format!("{}/{}", base.trim_end_matches('/'), file.path);
        if is_system_files(&file.path, false) {
            ui.label(&file.path).on_hover_text(SYSTEM_FILES_HINT);
        } else if ui
            .add(egui::Button::new(&file.path).truncate())
            .on_hover_text(&path)
            .clicked()
        {
            *reveal = Some(path);
        }
        ui.end_row();
    }
}

fn type_rows(ui: &mut egui::Ui, overview: &Overview, measure: MeasureView) {
    header(ui, "Endung", "Dateien");
    for total in &overview.types {
        ui.label(&total.extension);
        ui.label(format_bytes(total.size));
        ui.label(measure.allocated_text(total.allocated));
        ui.label(total.files.to_string());
        ui.end_row();
    }
}
