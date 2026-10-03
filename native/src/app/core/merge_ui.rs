use super::prelude::*;
use super::*;
use super::sync_merge_types::{MergeDecision, line_display};

impl App {
    pub(in crate::app) fn ui_merge(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.merge.take() else { return; };
        let busy = self.merge_load_rx.is_some() || self.merge_apply_rx.is_some();
        let mut open = true; let mut decision = None; let mut retry = false;
        egui::Window::new(format!("Zeilenvergleich: {}", m.rel)).open(&mut open)
            .collapsible(false).resizable(true).default_size([900.0, 600.0])
            .max_size(theme::window_content_limit(ctx)).constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| {
                if busy {
                    ui.horizontal(|ui| { ui.spinner(); ui.label("Originale laden oder sicher speichern…"); });
                    if ui.button("Abbrechen").clicked() { self.cancel_merge(); }
                    return;
                }
                if let Some(error) = &m.last_error { ui.colored_label(theme::warning(ui), error); }
                if m.retry.is_some() {
                    ui.label("Wiederholung mit den ursprünglichen Bytes und derselben Entscheidung.");
                    if ui.button("Erneut versuchen").clicked() { retry = true; }
                    return;
                }
                if let Some(error) = &m.text_error { ui.colored_label(theme::warning(ui), error); }
                ui.label("A = Quelle, B = Ziel. Bei geänderter Zeile genau eine Seite wählen.");
                ui.label("Das Zeilenformat stammt anfangs von A; „Alle B“ übernimmt auch dessen Zeilenformat.");
                ui.add_enabled_ui(m.text_error.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("Alle A").clicked() {
                            for r in &mut m.rows { r.take_left = r.left.is_some(); r.take_right = false; }
                            m.shape = m.shape_a;
                        }
                        if ui.small_button("Alle B").clicked() {
                            for r in &mut m.rows { r.take_right = r.right.is_some(); r.take_left = false; }
                            m.shape = m.shape_b;
                        }
                    });
                    let width = (ui.available_width() / 2.0 - 8.0).max(80.0);
                    egui::ScrollArea::vertical().max_height(420.0).show_rows(ui, 24.0, m.rows.len(), |ui, visible| {
                        for index in visible {
                            let row = &mut m.rows[index];
                            let conflict = !row.equal && row.left.is_some() && row.right.is_some();
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(egui::vec2(width,24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    if !row.equal {
                                        if conflict {
                                            if ui.selectable_label(row.take_left,"A").clicked() { row.take_left=true; row.take_right=false; }
                                        } else { ui.checkbox(&mut row.take_left, ""); }
                                    }
                                    ui.add(egui::Label::new(RichText::new(line_display(row.left.as_deref().unwrap_or("∅"))).monospace()).truncate());
                                });
                                ui.allocate_ui_with_layout(egui::vec2(width,24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    if !row.equal {
                                        if conflict {
                                            if ui.selectable_label(row.take_right,"B").clicked() { row.take_right=true; row.take_left=false; }
                                        } else { ui.checkbox(&mut row.take_right, ""); }
                                    }
                                    ui.add(egui::Label::new(RichText::new(line_display(row.right.as_deref().unwrap_or("∅"))).monospace()).truncate());
                                });
                            });
                        }
                    });
                    if ui.button("Zusammenführen und speichern").clicked() {
                        decision = Some(MergeDecision::Rows(m.shape));
                    }
                });
                let both = m.session.as_ref().is_some_and(|s| s.original_a.is_some() && s.original_b.is_some());
                if ui.add_enabled(both, egui::Button::new("Beide Originaldateien behalten (Name von A)"))
                    .on_hover_text("A behält den Namen, B bleibt als eigene Kopie auf beiden Seiten erhalten. Originalbytes bleiben unverändert.").clicked() {
                    decision = Some(MergeDecision::KeepBoth { keep_a:true });
                }
            });
        if retry || decision.is_some() { self.submit_merge(m, decision); }
        else if open || busy {
            if !open { self.cancel_merge(); }
            self.merge = Some(m);
        }
    }
}
