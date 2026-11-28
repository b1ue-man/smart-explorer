//! Settings page „Sprachpakete“: the languages of the text view
//! (`textview::LanguageStore::default_store`) with origin, extensions,
//! license, „überdeckt von“ and problems; adding a `.sublime-syntax` file or
//! a pack folder, removing added packs and opening the language folder.
//! Listing, adding and removing run on a background thread (the built-in
//! set is large).
use super::prelude::*;
use crate::textview::{LanguagePack, LanguageStore, PackOrigin, PackSource};

type ListResult = Result<(Vec<LanguagePack>, PathBuf), String>;

#[derive(Default)]
pub(in crate::app) struct LanguagesPage {
    list: Option<ListResult>,
    loading: Option<Receiver<ListResult>>,
    operation: Option<Receiver<Result<String, String>>>,
    message: Option<Result<String, String>>,
    filter: String,
}

fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<Receiver<T>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("language-packs".into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .ok()
        .map(|_| rx)
}

/// Whether a pack matches the filter text (name or extension).
pub(in crate::app) fn pack_matches(pack: &LanguagePack, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return true;
    }
    pack.name.to_lowercase().contains(&filter)
        || pack.languages.iter().any(|language| {
            language.name.to_lowercase().contains(&filter)
                || language.extensions.iter().any(|extension| {
                    extension.to_lowercase().trim_start_matches('.')
                        == filter.trim_start_matches('.')
                })
        })
}

impl LanguagesPage {
    fn reload(&mut self) {
        self.loading = spawn(|| {
            let store = LanguageStore::default_store();
            Ok((store.list(), store.folder().to_path_buf()))
        });
        if self.loading.is_none() {
            self.list = Some(Err("Die Liste konnte nicht gelesen werden.".to_string()));
        }
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.loading {
            if let Ok(result) = rx.try_recv() {
                self.list = Some(result);
                self.loading = None;
            }
        }
        if let Some(rx) = &self.operation {
            if let Ok(result) = rx.try_recv() {
                self.message = Some(result);
                self.operation = None;
                self.reload();
            }
        }
    }

    fn add(&mut self, path: PathBuf) {
        self.operation = spawn(move || {
            LanguageStore::default_store()
                .add(PackSource::Path(path), None)
                .map(|pack| format!("Sprachpaket „{}“ hinzugefügt.", pack.name))
                .map_err(|error| error.to_string())
        });
    }

    fn remove(&mut self, name: String) {
        self.operation = spawn(move || {
            LanguageStore::default_store()
                .remove(&name)
                .map(|()| format!("Sprachpaket „{name}“ entfernt."))
                .map_err(|error| error.to_string())
        });
    }

    pub(in crate::app) fn busy(&self) -> bool {
        self.loading.is_some() || self.operation.is_some()
    }
}

impl App {
    pub(in crate::app) fn ui_settings_languages(&mut self, ui: &mut egui::Ui) {
        let page = &mut self.text_tools.languages;
        page.poll();
        if page.list.is_none() && page.loading.is_none() {
            page.reload();
        }
        if page.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.label("Sprachen der Textansicht (Hervorhebung). Eigene Pakete im Sublime-Syntax-Format (.sublime-syntax) gehen den eingebauten vor.");
        let mut open_folder = None;
        let folder = match &page.list {
            Some(Ok((_, folder))) => Some(folder.clone()),
            _ => None,
        };
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(page.operation.is_none(), |ui| {
                ui.menu_button("Paket hinzufügen…", |ui| {
                    if ui.button("Datei (.sublime-syntax) …").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Sublime-Syntax", &["sublime-syntax"])
                            .pick_file()
                        {
                            page.add(path);
                        }
                    }
                    if ui.button("Ordner eines Pakets …").clicked() {
                        ui.close_menu();
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            page.add(path);
                        }
                    }
                });
            });
            if ui
                .add_enabled(folder.is_some(), egui::Button::new("Ordner öffnen"))
                .on_hover_text(
                    folder
                        .as_ref()
                        .map(|f| f.display().to_string())
                        .unwrap_or_default(),
                )
                .clicked()
            {
                open_folder = folder.clone();
            }
            if ui
                .add_enabled(!page.busy(), egui::Button::new("Neu laden"))
                .clicked()
            {
                page.reload();
            }
            if page.busy() {
                ui.spinner();
            }
        });
        match &page.message {
            Some(Ok(text)) => {
                ui.colored_label(super::theme::success(ui), text);
            }
            Some(Err(text)) => {
                ui.colored_label(super::theme::danger(ui), text);
            }
            None => {}
        }
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut page.filter)
                    .hint_text("Name oder Endung")
                    .desired_width(200.0),
            );
        });
        ui.separator();
        let mut remove = None;
        match &page.list {
            None => {
                ui.label("Liste wird gelesen …");
            }
            Some(Err(error)) => {
                ui.colored_label(super::theme::danger(ui), error);
            }
            Some(Ok((packs, _))) => {
                let shown: Vec<&LanguagePack> = packs
                    .iter()
                    .filter(|pack| pack_matches(pack, &page.filter))
                    .collect();
                ui.label(format!("{} von {} Einträgen", shown.len(), packs.len()));
                for pack in shown {
                    if pack_row(ui, pack, page.operation.is_none()) {
                        remove = Some(pack.name.clone());
                    }
                }
            }
        }
        if let Some(name) = remove {
            page.remove(name);
        }
        if let Some(folder) = open_folder {
            match std::fs::create_dir_all(&folder) {
                Ok(()) => self.open_path(&folder.to_string_lossy()),
                Err(error) => {
                    self.error_msg = Some(format!("Sprachordner anlegen: {error}"));
                }
            }
        }
    }
}

/// One entry; `true` when „Entfernen“ was clicked.
fn pack_row(ui: &mut egui::Ui, pack: &LanguagePack, can_remove: bool) -> bool {
    let muted = super::theme::muted(ui);
    let mut remove = false;
    ui.horizontal_wrapped(|ui| {
        ui.strong(&pack.name);
        let origin = match pack.origin {
            PackOrigin::Builtin => "eingebaut",
            PackOrigin::Added => "hinzugefügt",
        };
        ui.label(RichText::new(origin).color(muted));
        if let Some(by) = &pack.shadowed_by {
            ui.label(
                RichText::new(format!("überdeckt von „{by}“")).color(super::theme::warning(ui)),
            );
        }
        if pack.origin == PackOrigin::Added
            && ui
                .add_enabled(can_remove, egui::Button::new("Entfernen"))
                .clicked()
        {
            remove = true;
        }
    });
    let languages: Vec<String> = pack
        .languages
        .iter()
        .map(|language| {
            if language.extensions.is_empty() {
                language.name.clone()
            } else {
                format!("{} ({})", language.name, language.extensions.join(", "))
            }
        })
        .collect();
    if !languages.is_empty() {
        ui.label(RichText::new(languages.join(" · ")).small());
    }
    if !pack.license.is_empty() {
        ui.label(
            RichText::new(format!("Lizenz: {}", pack.license))
                .small()
                .color(muted),
        );
    }
    if let Some(path) = &pack.path {
        ui.label(
            RichText::new(path.display().to_string())
                .small()
                .color(muted),
        );
    }
    for problem in &pack.problems {
        ui.colored_label(super::theme::warning(ui), problem);
    }
    ui.separator();
    remove
}
