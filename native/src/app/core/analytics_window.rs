//! Read-only elevated analysis. Deliberately does not construct the ordinary App:
//! no daemon, updater, shell registration, persisted jobs or file-write actions.
use super::{analytics_access::issues_ui, analytics_accessibility::treemap_accessible_list,
    app_models::TmCell, treemap::{nested_treemap, TM_HEADER}};
use crate::{analytics::{self, AnalysisStartup, Progress, ScanOutcome, ScanStatus, SizeNode}, format::format_bytes};
use eframe::egui;
use std::sync::atomic::Ordering;

pub(crate) fn run_analysis_window(request: Result<AnalysisStartup, String>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1050.0, 760.0])
            .with_min_inner_size([650.0, 450.0]).with_title("Smart Explorer – Administrator-Speicheranalyse"),
        ..Default::default()
    };
    eframe::run_native("Smart Explorer – Administrator-Speicheranalyse", options, Box::new(move |cc| {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Ok(Box::new(AnalysisWindow::new(request)))
    }))
}

struct AnalysisWindow {
    root: String,
    admitted: bool,
    progress: Progress,
    rx: Option<crossbeam_channel::Receiver<ScanOutcome>>,
    outcome: Option<ScanOutcome>,
    focus: Vec<String>,
    cells: Vec<TmCell>,
    cells_rect: egui::Rect,
}

impl AnalysisWindow {
    fn new(request: Result<AnalysisStartup, String>) -> Self {
        let mut window = Self { root: String::new(), admitted: false, progress: Progress::default(),
            rx: None, outcome: None, focus: Vec::new(), cells: Vec::new(), cells_rect: egui::Rect::ZERO };
        match request.and_then(|request| {
            analytics::verify_analysis_startup(&request)?;
            Ok(request)
        }) {
            Ok(request) => {
                window.root = request.root.replace('\\', "/");
                window.admitted = true;
                window.start();
            }
            Err(error) => window.outcome = Some(ScanOutcome::failed("Administrator-Analyse", error)),
        }
        window
    }

    fn start(&mut self) {
        if !self.admitted || self.rx.is_some() { return; }
        self.progress = Progress::default();
        self.outcome = None;
        self.focus.clear();
        self.cells.clear();
        let progress = self.progress.clone();
        let root = self.root.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new().name("admin-storage-analytics".into()).spawn(move || {
            let outcome = analytics::scan(std::path::Path::new(&root), &progress);
            let _ = tx.send(outcome);
        }) {
            Ok(_) => self.rx = Some(rx),
            Err(error) => self.outcome = Some(ScanOutcome::failed(&self.root, error.to_string())),
        }
    }

    fn poll(&mut self) {
        match self.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(outcome)) => { self.rx = None; self.outcome = Some(outcome); self.cells.clear(); }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.rx = None;
                self.outcome = Some(ScanOutcome::failed(&self.root, "Analyse-Thread ohne Ergebnis beendet"));
            }
            _ => {}
        }
    }
}

impl Drop for AnalysisWindow {
    fn drop(&mut self) { self.progress.cancel.store(true, Ordering::Relaxed); }
}

impl eframe::App for AnalysisWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        let mut restart = false;
        let mut up = false;
        let mut drill = None;
        let mut selected_file = None;
        let base = if self.focus.is_empty() { self.root.clone() }
            else { format!("{}/{}", self.root.trim_end_matches('/'), self.focus.join("/")) };
        let node = focused(self.outcome.as_ref().and_then(|o| o.tree.as_ref()), &self.focus);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Speicheranalyse mit Administrator-Leserechten");
            ui.label("Nur Analyse: keine Änderungen an Dateien, Besitzrechten oder Berechtigungen.");
            ui.horizontal_wrapped(|ui| {
                up = ui.add_enabled(!self.focus.is_empty(), egui::Button::new("↑ Eine Ebene höher")).clicked();
                ui.label(&base);
                restart = ui.add_enabled(self.admitted && self.rx.is_none(), egui::Button::new("Neu scannen")).clicked();
            });
            if self.rx.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{} Dateien · {} Ordner · {}",
                        self.progress.files.load(Ordering::Relaxed), self.progress.dirs.load(Ordering::Relaxed),
                        format_bytes(self.progress.bytes.load(Ordering::Relaxed))));
                    if ui.button("Abbrechen").clicked() { self.progress.cancel.store(true, Ordering::Relaxed); }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
            if let Some(outcome) = &self.outcome {
                ui.label(match outcome.status {
                    ScanStatus::Complete => "Analyse abgeschlossen.",
                    ScanStatus::Partial => "Teilresultat: Nicht alle Pfade konnten gelesen werden; Details unten.",
                    ScanStatus::Failed => "Analyse fehlgeschlagen; Details unten.",
                    ScanStatus::Canceled => "Analyse abgebrochen.",
                });
                issues_ui(ui, &outcome.issues, outcome.suppressed_issues, outcome.permission_denied);
                if outcome.permission_denied > 0 {
                    ui.label("Sicherungsleserechte wurden verwendet. Verbleibende Sperren können vom Dateisystem oder Anbieter erzwungen werden; sie wurden nicht stillschweigend übersprungen.");
                }
            }
            if let Some(node) = node { ui.label(format_bytes(node.size)); }
            treemap_accessible_list(ui, node, &base, &mut drill, &mut selected_file);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ui.available_height().max(120.0)), egui::Sense::click());
            if rect != self.cells_rect || self.cells.is_empty() {
                self.cells.clear();
                if let Some(node) = node { nested_treemap(rect, node, base.trim_end_matches('/'), 0, None, &mut self.cells); }
                self.cells_rect = rect;
            }
            let painter = ui.painter_at(rect);
            for cell in &self.cells {
                painter.rect_filled(cell.rect, 1.0, if cell.container { cell.color.gamma_multiply(0.4) } else { cell.color });
                let label_rect = if cell.container { egui::Rect::from_min_max(cell.rect.min,
                    egui::pos2(cell.rect.max.x, cell.rect.min.y + TM_HEADER)) } else { cell.rect };
                if label_rect.width() > 40.0 && label_rect.height() >= 15.0 {
                    painter.with_clip_rect(label_rect.shrink(2.0)).text(label_rect.min + egui::vec2(3.0, 1.0),
                        egui::Align2::LEFT_TOP, format!("{}  {}", cell.name, format_bytes(cell.size)),
                        egui::FontId::proportional(11.0), egui::Color32::WHITE);
                }
            }
            if let Some(pos) = response.hover_pos() {
                if let Some(cell) = self.cells.iter().rev().find(|cell| cell.rect.contains(pos)) {
                    let clicked = response.clicked();
                    response.on_hover_text(format!("{}\n{}", cell.path, format_bytes(cell.size)));
                    if clicked && cell.is_dir { drill = Some(cell.path.clone()); }
                }
            }
        });
        if restart { self.start(); }
        else if up { self.focus.pop(); self.cells.clear(); }
        else if let Some(path) = drill {
            if let Some(relative) = path.strip_prefix(self.root.trim_end_matches('/'))
                .and_then(|relative| relative.strip_prefix('/')) {
                self.focus = relative.split('/').map(str::to_string).collect();
                self.cells.clear();
            }
        }
        // File selection is deliberately not an elevated file-open action.
        if let Some(path) = selected_file { ctx.copy_text(path); }
    }
}

fn focused<'a>(mut node: Option<&'a SizeNode>, focus: &[String]) -> Option<&'a SizeNode> {
    for part in focus { node = node?.children.iter().find(|child| child.is_dir && &*child.name == part.as_str()); }
    node
}
