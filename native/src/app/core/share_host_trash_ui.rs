//! The local host restore consumer; remote peers never read this catalog.
use super::*;
use crate::host_trash::{CatalogPage, RestoreOutcome};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct View {
    page: Arc<CatalogPage>,
    cursor: Option<String>,
    loaded: bool,
    busy: bool,
    message: Option<String>,
}
enum Action {
    Load(Option<String>),
    Restore(String),
}

impl App {
    pub(super) fn ui_share_host_trash(&mut self, ui: &mut egui::Ui) {
        show(ui);
    }
}

fn show(ui: &mut egui::Ui) {
    ui.heading("Smart-Explorer-Papierkorb");
    if !crate::host_trash::available() {
        ui.label("Diese Host-Wiederherstellung ist auf Windows verfügbar.");
        return;
    }
    ui.add(egui::Label::new("Hier finden Sie Dateien, die ein freigegebenes Gerät auf diesem Host in den Smart-Explorer-Papierkorb verschoben hat. Wiederherstellen ersetzt keine vorhandene Datei.").wrap());
    let key = egui::Id::new("host-share-trash");
    let state = ui.ctx().data_mut(|data| {
        if let Some(state) = data.get_temp::<Arc<Mutex<View>>>(key) {
            state
        } else {
            let state = Arc::new(Mutex::new(View::default()));
            data.insert_temp(key, state.clone());
            state
        }
    });
    let (page, busy, loaded, message) = {
        let view = state.lock().unwrap_or_else(|error| error.into_inner());
        (
            view.page.clone(),
            view.busy,
            view.loaded,
            view.message.clone(),
        )
    };
    let mut action = (!loaded && !busy).then_some(Action::Load(None));
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("Aktualisieren"))
            .clicked()
        {
            action = Some(Action::Load(None));
        }
        if let Some(next) = &page.next {
            if ui
                .add_enabled(!busy, egui::Button::new("Weitere Einträge"))
                .clicked()
            {
                action = Some(Action::Load(Some(next.clone())));
            }
        }
        if busy {
            ui.spinner();
            ui.label("Papierkorb wird bearbeitet…");
        }
    });
    if let Some(message) = message {
        ui.add(egui::Label::new(message).wrap());
    }
    for issue in &page.issues {
        ui.add(egui::Label::new(issue).wrap());
    }
    if page.suppressed_issues > 0 {
        ui.label(format!(
            "{} weitere Eintragsprobleme; bitte erneut aktualisieren.",
            page.suppressed_issues
        ));
    }
    if loaded && !busy && page.entries.is_empty() {
        ui.label("Keine Einträge auf dieser Seite.");
    }
    for entry in &page.entries {
        ui.push_id(&entry.id, |ui| {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(entry.state.label()).strong());
                    if entry.can_restore()
                        && ui
                            .add_enabled(!busy, egui::Button::new("Wiederherstellen"))
                            .clicked()
                    {
                        action = Some(Action::Restore(entry.id.clone()));
                    }
                });
                ui.add(egui::Label::new(&entry.original).wrap());
                ui.horizontal_wrapped(|ui| {
                    if let Some(size) = entry.size {
                        ui.label(format!("{size} Bytes"));
                    }
                    if let Some(when) = entry
                        .created_ms
                        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                    {
                        ui.label(
                            when.with_timezone(&chrono::Local)
                                .format("%d.%m.%Y %H:%M")
                                .to_string(),
                        );
                    }
                });
                if let Some(detail) = &entry.detail {
                    ui.add(egui::Label::new(detail).wrap());
                }
            });
        });
    }
    if let Some(action) = action {
        start(state, ui.ctx().clone(), action);
    }
}

fn start(state: Arc<Mutex<View>>, ctx: egui::Context, action: Action) {
    let cursor = {
        let mut view = state.lock().unwrap_or_else(|error| error.into_inner());
        if view.busy {
            return;
        }
        view.busy = true;
        match &action {
            Action::Load(cursor) => cursor.clone(),
            Action::Restore(_) => view.cursor.clone(),
        }
    };
    let result_state = state.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("host-trash-ui".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let message = match action {
                    Action::Load(_) => None,
                    Action::Restore(id) => Some(match crate::host_trash::restore(&id) {
                        Ok(RestoreOutcome::Restored) => "Datei wiederhergestellt.".to_owned(),
                        Ok(RestoreOutcome::AlreadyAtOriginal) => {
                            "Die Datei liegt bereits am Originalort.".to_owned()
                        }
                        Err(error) => format!("Wiederherstellen: {error}"),
                    }),
                };
                (crate::host_trash::list(cursor.as_deref()), message)
            }));
            let mut view = result_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            view.busy = false;
            view.loaded = true;
            match result {
                Ok((Ok(page), message)) => {
                    view.page = Arc::new(page);
                    view.cursor = cursor;
                    view.message = message;
                }
                Ok((Err(error), message)) => {
                    view.message = Some(format!(
                        "{}Papierkorb aktualisieren: {error}",
                        message.map_or(String::new(), |text| format!("{text} "))
                    ));
                }
                Err(_) => {
                    view.message = Some(
                        "Papierkorb konnte nicht bearbeitet werden; bitte erneut versuchen.".into(),
                    )
                }
            }
            drop(view);
            repaint.request_repaint();
        });
    if let Err(error) = spawned {
        let mut view = state.lock().unwrap_or_else(|error| error.into_inner());
        view.busy = false;
        view.loaded = true;
        view.message = Some(format!("Papierkorb starten: {error}"));
        ctx.request_repaint();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn review_task_host_trash_page_exposes_visible_restore_consumer() {
        let ctx = egui::Context::default();
        let page = CatalogPage {
            entries: vec![crate::host_trash::CatalogEntry {
                id: "00000000000000000000000000000001".into(),
                original: "C:\\source\\copy".into(),
                size: Some(4),
                created_ms: None,
                state: crate::host_trash::EntryState::Held,
                detail: None,
            }],
            ..Default::default()
        };
        let view = Arc::new(Mutex::new(View {
            page: Arc::new(page),
            loaded: true,
            ..Default::default()
        }));
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("host-share-trash"), view));
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, show);
        });
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Smart-Explorer-Papierkorb"));
        assert!(text.contains("C:\\source\\copy"));
        assert!(text.contains("Wiederherstellen"));
    }
}
