//! Live view of one sync job's log (`bisync::read_job_log`): new lines of a
//! running job appear within half a second, from every runner (background
//! service, this window, Android, terminal).
use super::*;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Lines kept in the view (rendering and memory bound); the file keeps more.
const MAX_VIEW_LINES: usize = 50_000;
const REFRESH: Duration = Duration::from_millis(500);

struct LogView {
    id: String,
    name: String,
    lines: Vec<String>,
    next: Option<u64>,
    size: u64,
    follow: bool,
    verbose: bool,
    filter: String,
    error: Option<String>,
    read_at: Option<Instant>,
}

fn slot(ctx: &egui::Context) -> Option<Arc<Mutex<LogView>>> {
    ctx.data(|data| data.get_temp::<Arc<Mutex<LogView>>>(egui::Id::new("sync-job-log-view")))
}

/// Opens (or switches) the log window for job `id`.
pub(in crate::app) fn open(ctx: &egui::Context, id: &str, name: &str) {
    let view = LogView {
        id: id.to_string(),
        name: name.to_string(),
        lines: Vec::new(),
        next: None,
        size: 0,
        follow: true,
        verbose: crate::bisync::job_log_verbose(id),
        filter: String::new(),
        error: None,
        read_at: None,
    };
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("sync-job-log-view"),
            Arc::new(Mutex::new(view)),
        )
    });
}

fn refresh(view: &mut LogView) {
    if view.read_at.is_some_and(|at| at.elapsed() < REFRESH) {
        return;
    }
    view.read_at = Some(Instant::now());
    match crate::bisync::read_job_log(&view.id, view.next) {
        Ok(chunk) => {
            if chunk.restarted {
                view.lines.clear();
            }
            view.lines.extend(chunk.text.lines().map(str::to_string));
            if view.lines.len() > MAX_VIEW_LINES {
                let excess = view.lines.len() - MAX_VIEW_LINES;
                view.lines.drain(..excess);
            }
            view.next = Some(chunk.next);
            view.size = chunk.size;
            view.error = None;
        }
        Err(error) => view.error = Some(error.to_string()),
    }
}

fn color(ui: &egui::Ui, line: &str) -> egui::Color32 {
    let tag = line
        .get(24..)
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("");
    match tag {
        "Fehler" | "Stopp" => theme::danger(ui),
        "Unterbrochen" | "Verschoben" | "Ausgelassen" | "Gestoppt" | "Wiederholung" => {
            theme::warning(ui)
        }
        "Aktion" => theme::success(ui),
        "Ergebnis" | "Ende" | "Start" | "Auslöser" | "Vorschau" => theme::accent(ui),
        _ => ui.visuals().text_color(),
    }
}

/// Draws the log window while one is open.
pub(in crate::app) fn show(ctx: &egui::Context) {
    let Some(shared) = slot(ctx) else {
        return;
    };
    let mut view = shared.lock().unwrap_or_else(|e| e.into_inner());
    refresh(&mut view);
    let mut open = true;
    let title = format!("Sync-Protokoll – {}", view.name);
    egui::Window::new(title)
        .id(egui::Id::new("sync-job-log-window"))
        .open(&mut open)
        .default_size([900.0, 520.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut view.follow, "Mitlaufen")
                    .on_hover_text("Immer die neuesten Zeilen zeigen");
                let mut verbose = view.verbose;
                if ui
                    .checkbox(&mut verbose, "Unveränderte Einträge einzeln protokollieren")
                    .on_hover_text(
                        "Schreibt ab dem nächsten Lauf auch jeden Vergleich ohne Änderung als eigene Zeile (große Ordner: viele Zeilen).",
                    )
                    .changed()
                {
                    match crate::bisync::set_job_log_verbose(&view.id, verbose) {
                        Ok(()) => view.verbose = verbose,
                        Err(error) => view.error = Some(format!("Einstellung speichern: {error}")),
                    }
                }
                ui.label("Filter:");
                ui.add(egui::TextEdit::singleline(&mut view.filter).desired_width(180.0));
                if let Some(path) = crate::bisync::job_log_path(&view.id) {
                    if ui.button("In Editor öffnen").clicked() {
                        open_local_path(&path.to_string_lossy(), OpenMode::Default);
                    }
                    if ui.button("Ordner zeigen").clicked() {
                        reveal_path_in_file_manager(&path.to_string_lossy());
                    }
                }
            });
            let shown = view.lines.len();
            let status = match &view.error {
                Some(error) => format!("Protokoll nicht lesbar: {error}"),
                None if shown == 0 => {
                    "Noch keine Einträge. Der nächste Lauf dieses Syncs schreibt hier jeden Schritt.".into()
                }
                None => format!(
                    "{shown} Zeilen angezeigt · Datei {:.1} MB · wird alle 0,5 s aktualisiert",
                    view.size as f64 / (1024.0 * 1024.0)
                ),
            };
            ui.colored_label(
                if view.error.is_some() {
                    theme::danger(ui)
                } else {
                    theme::muted(ui)
                },
                status,
            );
            ui.separator();
            let needle = view.filter.to_lowercase();
            let rows: Vec<usize> = if needle.is_empty() {
                (0..view.lines.len()).collect()
            } else {
                view.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| line.to_lowercase().contains(&needle))
                    .map(|(index, _)| index)
                    .collect()
            };
            let height = ui.text_style_height(&egui::TextStyle::Monospace);
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(view.follow)
                .show_rows(ui, height, rows.len(), |ui, range| {
                    for row in &rows[range] {
                        let line = &view.lines[*row];
                        ui.label(
                            egui::RichText::new(line.as_str())
                                .monospace()
                                .color(color(ui, line)),
                        );
                    }
                });
        });
    drop(view);
    if open {
        ctx.request_repaint_after(REFRESH);
    } else {
        ctx.data_mut(|data| data.remove::<Arc<Mutex<LogView>>>(egui::Id::new("sync-job-log-view")));
    }
}
