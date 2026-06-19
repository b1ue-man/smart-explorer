use super::prelude::*;
use super::*;

impl App {
    /// The line-merge window: a synced, side-by-side (git-diff-like) view of both
    /// versions; tick the line(s) from each side to keep in the merged result.
    pub(in crate::app) fn ui_merge(&mut self, ctx: &egui::Context) {
        let mut m = match self.merge.take() {
            Some(m) => m,
            None => return,
        };
        let loading = self.merge_load_rx.is_some();
        let mut open = true;
        let mut save = false;
        let mut keep_both_files = false;
        egui::Window::new(format!("⇄ Zeilenvergleich: {}", m.rel))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([900.0, 600.0])
            .show(ctx, |ui| {
                if loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Lade beide Versionen…");
                    });
                    return;
                }
                ui.label(
                    RichText::new("A = Quelle (links), B = Ziel (rechts). Gleiche Zeile auf beiden Seiten = Konflikt → genau EINE Seite wählen (Zeilen werden nicht zusammengefügt). Nur-eine-Seite-Zeilen kannst du einzeln übernehmen/weglassen.")
                        .small()
                        .color(Color32::from_gray(150)),
                );
                ui.horizontal(|ui| {
                    if ui.small_button("Alle A").clicked() {
                        for r in m.rows.iter_mut().filter(|r| !r.equal) { r.take_left = r.left.is_some(); r.take_right = false; }
                    }
                    if ui.small_button("Alle B").clicked() {
                        for r in m.rows.iter_mut().filter(|r| !r.equal) { r.take_right = r.right.is_some(); r.take_left = false; }
                    }
                });
                ui.separator();
                let gray = Color32::from_gray(150);
                let green = Color32::from_rgb(120, 200, 120);
                let blue = Color32::from_rgb(120, 180, 230);
                let colw = ((ui.available_width() - 40.0) / 2.0).max(120.0);
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("merge_grid").num_columns(2).striped(true).min_col_width(colw).show(ui, |ui| {
                        for r in m.rows.iter_mut() {
                            // Same line changed on BOTH sides = a real conflict:
                            // exactly one side may be taken (never concatenated).
                            let conflict = !r.equal && r.left.is_some() && r.right.is_some();
                            // Left (A) cell.
                            ui.horizontal(|ui| {
                                if r.equal {
                                    ui.add_space(20.0);
                                    ui.label(RichText::new(r.left.clone().unwrap_or_default()).monospace().color(gray));
                                } else if let Some(l) = r.left.clone() {
                                    if conflict {
                                        if ui.selectable_label(r.take_left, "A").on_hover_text("Diese Seite übernehmen").clicked() {
                                            r.take_left = true;
                                            r.take_right = false;
                                        }
                                    } else {
                                        ui.checkbox(&mut r.take_left, "");
                                    }
                                    ui.label(RichText::new(l).monospace().color(green));
                                } else {
                                    ui.add_space(20.0);
                                    ui.label(RichText::new("∅").monospace().color(Color32::from_gray(90)));
                                }
                            });
                            // Right (B) cell.
                            ui.horizontal(|ui| {
                                if r.equal {
                                    ui.add_space(20.0);
                                    ui.label(RichText::new(r.right.clone().unwrap_or_default()).monospace().color(gray));
                                } else if let Some(l) = r.right.clone() {
                                    if conflict {
                                        if ui.selectable_label(r.take_right, "B").on_hover_text("Diese Seite übernehmen").clicked() {
                                            r.take_right = true;
                                            r.take_left = false;
                                        }
                                    } else {
                                        ui.checkbox(&mut r.take_right, "");
                                    }
                                    ui.label(RichText::new(l).monospace().color(blue));
                                } else {
                                    ui.add_space(20.0);
                                    ui.label(RichText::new("∅").monospace().color(Color32::from_gray(90)));
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("✔ Zusammenführen & speichern").clicked() {
                        save = true;
                    }
                    if ui
                        .button("Beide als getrennte Dateien")
                        .on_hover_text("Kein Zusammenführen: A behält den Namen, B wird als „(Konflikt …)“-Kopie auf beiden Seiten gespeichert")
                        .clicked()
                    {
                        keep_both_files = true;
                    }
                });
            });
        if save {
            let merged = crate::linemerge::assemble_rows(&m.rows);
            self.start_merge_apply(m.rel.clone(), merged);
            self.merge = None; // close; result lands via drain
        } else if keep_both_files {
            let a_full = crate::linemerge::side_a(&m.rows);
            let b_full = crate::linemerge::side_b(&m.rows);
            self.start_merge_keep_both(m.rel.clone(), a_full, b_full);
            self.merge = None;
        } else if open {
            self.merge = Some(m);
        }
        // !open → leave closed (m dropped)
    }

    // ─── View ───────────────────────────────────────────────────────────

}
