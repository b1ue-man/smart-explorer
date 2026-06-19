use super::prelude::*;
use super::*;

impl App {
    /// The compare-result window: per-file differences, grouped by direction.
    pub(in crate::app) fn ui_preview(&mut self, ctx: &egui::Context) {
        let mut open = self.show_preview;
        // Set when the user clicks a row's "▶" to sync just that one file.
        let mut sync_one: Option<crate::bisync::Action> = None;
        egui::Window::new("🔍 Vergleich (Vorschau)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([680.0, 460.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(&self.preview_title)
                        .small()
                        .color(Color32::from_gray(170)),
                );
                ui.separator();
                if self.preview_running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Vergleiche beide Seiten…");
                        if ui.button("⏹ Stop").clicked() {
                            if let Some(c) = &self.preview_cancel {
                                c.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    });
                    return;
                }
                let p = match &self.preview {
                    Some(p) => p,
                    None => {
                        ui.label("—");
                        return;
                    }
                };
                if let Some(e) = &p.error {
                    ui.colored_label(Color32::from_rgb(230, 120, 120), format!("Fehler: {}", e));
                    return;
                }
                let mut to_b = 0usize;
                let mut to_a = 0usize;
                let mut del = 0usize;
                for act in &p.actions {
                    match act {
                        crate::bisync::Action::CopyAtoB(_)
                        | crate::bisync::Action::KeepBothAtoB(_) => to_b += 1,
                        crate::bisync::Action::CopyBtoA(_)
                        | crate::bisync::Action::KeepBothBtoA(_) => to_a += 1,
                        crate::bisync::Action::DeleteA(_)
                        | crate::bisync::Action::DeleteB(_) => del += 1,
                    }
                }
                ui.label(format!(
                    "Quelle: {} Dateien · Ziel: {} Dateien",
                    p.a_files, p.b_files
                ));
                ui.label(
                    RichText::new(format!(
                        "{}→ zum Ziel · {}← zur Quelle · {} zu löschen · {} Konflikte",
                        to_b,
                        to_a,
                        del,
                        p.conflicts.len()
                    ))
                    .strong(),
                );
                if p.actions.is_empty() && p.conflicts.is_empty() {
                    ui.add_space(6.0);
                    ui.colored_label(
                        Color32::from_rgb(120, 200, 120),
                        "✓ Beide Seiten sind im Einklang — nichts zu tun.",
                    );
                    return;
                }
                ui.label(
                    RichText::new("▶ neben einer Zeile synchronisiert nur diese eine Datei.")
                        .small()
                        .color(Color32::from_gray(130)),
                );
                ui.separator();
                let busy = self.apply_one_rx.is_some();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for c in &p.conflicts {
                        ui.colored_label(
                            Color32::from_rgb(230, 200, 90),
                            format!("⚠ Konflikt: {}", c.rel),
                        );
                    }
                    for act in &p.actions {
                        let (sym, color, rel) = match act {
                            crate::bisync::Action::CopyAtoB(r) => ("→", Color32::from_rgb(120, 200, 120), r),
                            crate::bisync::Action::CopyBtoA(r) => ("←", Color32::from_rgb(120, 200, 120), r),
                            crate::bisync::Action::DeleteB(r) => ("🗑→", Color32::from_rgb(230, 150, 120), r),
                            crate::bisync::Action::DeleteA(r) => ("🗑←", Color32::from_rgb(230, 150, 120), r),
                            crate::bisync::Action::KeepBothAtoB(r) => ("⇄→", Color32::from_rgb(230, 200, 90), r),
                            crate::bisync::Action::KeepBothBtoA(r) => ("⇄←", Color32::from_rgb(230, 200, 90), r),
                        };
                        ui.horizontal(|ui| {
                            if !busy && ui.small_button("▶").on_hover_text("Nur diese Datei jetzt synchronisieren").clicked() {
                                sync_one = Some(act.clone());
                            }
                            ui.colored_label(color, format!("{}  {}", sym, rel));
                        });
                    }
                });
            });
        self.show_preview = open;
        if let Some(act) = sync_one {
            // Optimistically drop it from the list and apply just that one file.
            if let Some(p) = self.preview.as_mut() {
                p.actions.retain(|a| a != &act);
            }
            if let Some(job_id) = self.preview_job_id.clone() {
                self.apply_one_action(job_id, act);
            }
        }
    }

    pub(in crate::app) fn drain_bisync(&mut self) {
        let out = match self.bisync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(o) => o,
            None => return,
        };
        self.bisync_rx = None;
        self.bisync_running = false;
        self.bisync_cancel = None;
        // Stamp the saved job (if this run came from one) so its schedule and
        // "last run" reflect reality, then refresh the cached list.
        if let Some(id) = self.running_job.take() {
            crate::syncjobs::mark_run(&id);
            let note = if out.errors.iter().any(|(k, _)| k == "abgebrochen") {
                "abgebrochen"
            } else if !out.errors.is_empty() {
                "Fehler"
            } else if !out.conflicts.is_empty() {
                "Konflikte"
            } else {
                "ok"
            };
            crate::syncjobs::record_result(
                &id,
                &crate::syncjobs::JobResult {
                    when: now_secs_i64(),
                    a_to_b: out.stats.a_to_b,
                    b_to_a: out.stats.b_to_a,
                    deleted: out.stats.deleted,
                    conflicts: out.conflicts.len() as u64,
                    errors: out.errors.len() as u64,
                    note: note.into(),
                },
            );
            self.sync_jobs = crate::syncjobs::load();
        }
        if let Some(ctx) = self.bisync_ctx.as_mut() {
            ctx.baseline = out.baseline;
        }
        self.bisync_conflicts = out.conflicts;
        let s = out.stats;
        if !out.errors.is_empty() {
            self.error_msg = Some(format!("Sync: {} Fehler", out.errors.len()));
        }
        self.notice = Some((
            format!(
                "⇄ Sync: {} →, {} ←, {} gelöscht, {} Konflikte ({} MB)",
                s.a_to_b,
                s.b_to_a,
                s.deleted,
                self.bisync_conflicts.len(),
                s.bytes / 1_048_576
            ),
            std::time::Instant::now(),
        ));
        if !self.bisync_conflicts.is_empty() {
            self.show_bisync_conflicts = true;
        }
        // The current view may have changed on disk.
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }

    /// Resolve conflict `idx` by keeping side A (→ overwrites B) or side B.
    pub(in crate::app) fn resolve_conflict(&mut self, idx: usize, keep_a: bool) {
        if idx >= self.bisync_conflicts.len() {
            return;
        }
        let rel = self.bisync_conflicts[idx].rel.clone();
        let ctx = match self.bisync_ctx.as_mut() {
            Some(c) => c,
            None => return,
        };
        match crate::bisync::resolve(
            &*ctx.a, &ctx.root_a, &*ctx.b, &ctx.root_b, &rel, keep_a, &ctx.pair,
        ) {
            Ok((sa, sb)) => {
                ctx.baseline.insert(rel, (sa, sb));
            }
            Err(e) => {
                self.error_msg = Some(format!("Konfliktlösung: {}", e));
                return;
            }
        }
        self.bisync_conflicts.remove(idx);
        if self.bisync_conflicts.is_empty() {
            self.finish_bisync_conflicts();
        }
    }

    /// Persist the updated baseline once all conflicts are handled.
    pub(in crate::app) fn finish_bisync_conflicts(&mut self) {
        if let Some(ctx) = &self.bisync_ctx {
            let path = crate::bisync::baseline_path(&ctx.pair);
            let _ = crate::bisync::save_baseline(&path, &ctx.baseline);
        }
        self.show_bisync_conflicts = false;
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }

    pub(in crate::app) fn ui_bisync_conflicts(&mut self, ctx: &egui::Context) {
        if !self.show_bisync_conflicts {
            return;
        }
        if self.bisync_conflicts.is_empty() {
            self.finish_bisync_conflicts();
            return;
        }
        let mut keep_a: Option<usize> = None;
        let mut keep_b: Option<usize> = None;
        let mut skip: Option<usize> = None;
        let mut merge_req: Option<usize> = None;
        let mut close = false;
        let mut all_a = false;
        let mut all_b = false;
        let conflicts = self.bisync_conflicts.clone();
        egui::Window::new(format!("⚠ Sync-Konflikte ({})", conflicts.len()))
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Beide Seiten wurden geändert. Wähle, welche Version gilt — die andere wird vorher reversibel gesichert.");
                ui.horizontal(|ui| {
                    if ui.button("Alle: ← A behalten").clicked() { all_a = true; }
                    if ui.button("Alle: B behalten →").clicked() { all_b = true; }
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, c) in conflicts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let a = c.a.map(|s| format!("{} B, {}", s.size, fmt_ms(s.mtime_ms))).unwrap_or_else(|| "—".into());
                            let b = c.b.map(|s| format!("{} B, {}", s.size, fmt_ms(s.mtime_ms))).unwrap_or_else(|| "—".into());
                            if ui.small_button("← A").on_hover_text(format!("A: {a}")).clicked() { keep_a = Some(i); }
                            if ui.small_button("B →").on_hover_text(format!("B: {b}")).clicked() { keep_b = Some(i); }
                            if ui.small_button("⇄ Zeilen").on_hover_text("Zeilenweise zusammenführen").clicked() { merge_req = Some(i); }
                            if ui.small_button("⏭").on_hover_text("Vorerst überspringen").clicked() { skip = Some(i); }
                            ui.label(&c.rel);
                        });
                    }
                });
                ui.add_space(6.0);
                if ui.button("Schließen (Rest später)").clicked() { close = true; }
            });
        if all_a || all_b {
            // resolve all in index order; removals shrink the vec, so resolve 0 repeatedly
            while !self.bisync_conflicts.is_empty() {
                self.resolve_conflict(0, all_a);
            }
        } else if let Some(i) = keep_a {
            self.resolve_conflict(i, true);
        } else if let Some(i) = keep_b {
            self.resolve_conflict(i, false);
        } else if let Some(i) = skip {
            if i < self.bisync_conflicts.len() {
                self.bisync_conflicts.remove(i);
            }
            if self.bisync_conflicts.is_empty() {
                self.finish_bisync_conflicts();
            }
        } else if let Some(i) = merge_req {
            if let Some(c) = conflicts.get(i) {
                self.start_merge(c.rel.clone());
            }
        }
        if close {
            self.finish_bisync_conflicts();
        }
    }

    /// Begin a line-merge for one conflict: read both versions off-thread, diff.
    pub(in crate::app) fn start_merge(&mut self, rel: String) {
        let ctx = match &self.bisync_ctx {
            Some(c) => c,
            None => return,
        };
        let (a, ra, b, rb) = (ctx.a.clone(), ctx.root_a.clone(), ctx.b.clone(), ctx.root_b.clone());
        let rel_t = rel.clone();
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("merge-load".into())
            .spawn(move || {
                let res = (|| -> Result<(String, Vec<crate::linemerge::Row>), String> {
                    let ta = read_text(&*a, &ep_join(&ra, &rel_t))?;
                    let tb = read_text(&*b, &ep_join(&rb, &rel_t))?;
                    Ok((rel_t.clone(), crate::linemerge::rows(&ta, &tb)))
                })();
                let _ = tx.send(res);
            })
            .ok();
        self.merge_load_rx = Some(rx);
        self.merge = Some(MergeUi { rel, rows: Vec::new() }); // shows "loading"
    }

    pub(in crate::app) fn drain_merge(&mut self) {
        if let Some(res) = self.merge_load_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.merge_load_rx = None;
            match res {
                Ok((rel, rows)) => self.merge = Some(MergeUi { rel, rows }),
                Err(e) => {
                    self.error_msg = Some(format!("Zusammenführen: {}", e));
                    self.merge = None;
                }
            }
        }
        if let Some(res) = self.merge_apply_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.merge_apply_rx = None;
            match res {
                Ok((rel, sa, sb)) => {
                    if let Some(ctx) = self.bisync_ctx.as_mut() {
                        ctx.baseline.insert(rel.clone(), (Some(sa), Some(sb)));
                    }
                    self.bisync_conflicts.retain(|c| c.rel != rel);
                    self.notice =
                        Some((format!("✓ „{}“ zusammengeführt", rel), std::time::Instant::now()));
                    if self.bisync_conflicts.is_empty() {
                        self.finish_bisync_conflicts();
                    }
                }
                Err(e) => self.error_msg = Some(format!("Zusammenführen: {}", e)),
            }
        }
    }

    /// Write the merged text to both sides off-thread, then resolve the conflict.
    pub(in crate::app) fn start_merge_apply(&mut self, rel: String, merged: String) {
        let ctx = match &self.bisync_ctx {
            Some(c) => c,
            None => return,
        };
        let (a, ra, b, rb) = (ctx.a.clone(), ctx.root_a.clone(), ctx.b.clone(), ctx.root_b.clone());
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("merge-apply".into())
            .spawn(move || {
                let res = (|| -> Result<(String, crate::bisync::Sig, crate::bisync::Sig), String> {
                    let pa = ep_join(&ra, &rel);
                    let pb = ep_join(&rb, &rel);
                    write_bytes(&*a, &pa, merged.as_bytes())?;
                    write_bytes(&*b, &pb, merged.as_bytes())?;
                    Ok((rel.clone(), sig_from(&*a, &pa), sig_from(&*b, &pb)))
                })();
                let _ = tx.send(res);
            })
            .ok();
        self.merge_apply_rx = Some(rx);
    }

    /// Resolve a conflict by keeping BOTH versions as separate files: A keeps the
    /// original name on both sides; B is written as a "(Konflikt …)" sibling on
    /// both sides. No line concatenation. Reuses the merge-apply result channel.
    pub(in crate::app) fn start_merge_keep_both(&mut self, rel: String, a_full: String, b_full: String) {
        let ctx = match &self.bisync_ctx {
            Some(c) => c,
            None => return,
        };
        let (a, ra, b, rb) = (ctx.a.clone(), ctx.root_a.clone(), ctx.b.clone(), ctx.root_b.clone());
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("merge-keepboth".into())
            .spawn(move || {
                let res = (|| -> Result<(String, crate::bisync::Sig, crate::bisync::Sig), String> {
                    let crel = conflict_rel_name(&rel);
                    let pa = ep_join(&ra, &rel);
                    let pb = ep_join(&rb, &rel);
                    write_bytes(&*a, &pa, a_full.as_bytes())?;
                    write_bytes(&*b, &pb, a_full.as_bytes())?;
                    write_bytes(&*a, &ep_join(&ra, &crel), b_full.as_bytes())?;
                    write_bytes(&*b, &ep_join(&rb, &crel), b_full.as_bytes())?;
                    Ok((rel.clone(), sig_from(&*a, &pa), sig_from(&*b, &pb)))
                })();
                let _ = tx.send(res);
            })
            .ok();
        self.merge_apply_rx = Some(rx);
    }

}
