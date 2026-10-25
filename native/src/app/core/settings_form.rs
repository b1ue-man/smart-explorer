//! Shared responsive settings rows. All labels in one form are measured
//! together; the same content stacks when two readable columns do not fit.
use eframe::egui::{self, RichText};
use crate::app::theme;

pub(super) struct Form {
    id: egui::Id,
    label_width: f32,
    stacked: bool,
    row: usize,
}
impl Form {
    pub(super) fn new(ui: &egui::Ui, salt: impl std::hash::Hash, labels: &[&str]) -> Self {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let width = labels.iter().map(|label| {
            ui.painter().layout_no_wrap((*label).into(), font.clone(), ui.visuals().text_color()).size().x
        }).fold(0.0_f32, f32::max);
        let spacing = ui.spacing().item_spacing.x;
        let available = ui.available_width();
        let label_width = width.min(available * 0.44);
        let control_minimum = font.size * 16.0;
        Self { id: ui.id().with(salt), label_width, stacked: available < label_width + spacing + control_minimum, row: 0 }
    }
    pub(super) fn row<R>(&mut self, ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
        self.row_labelled(ui, label, |ui, _| contents(ui))
    }
    pub(super) fn row_labelled<R>(&mut self, ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui, egui::Id) -> R) -> R {
        self.row += 1;
        ui.push_id(self.id.with(self.row), |ui| {
            if self.stacked {
                ui.vertical(|ui| { let label = ui.label(label); contents(ui, label.id) }).inner
            } else {
                ui.horizontal(|ui| {
                    let label = ui.add_sized([self.label_width, 0.0], egui::Label::new(label).wrap());
                    ui.vertical(|ui| contents(ui, label.id)).inner
                }).inner
            }
        }).inner
    }

}

pub(super) fn status(ui: &mut egui::Ui, busy: bool, message: Option<&str>, error: Option<&str>) {
    ui.horizontal_wrapped(|ui| {
        if busy { ui.spinner(); ui.label("Wird gespeichert …"); }
        if let Some(error) = error { ui.colored_label(theme::danger(ui), error); }
        else if let Some(message) = message { ui.label(RichText::new(message).color(theme::muted(ui))); }
    });
}
