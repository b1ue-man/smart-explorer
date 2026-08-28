use super::prelude::*;
use super::*;

/// The exact remote surface that received a secondary click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextTarget {
    Row { entry_idx: usize },
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextEntryKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteRowSelection {
    ClickedOnly,
    MultipleIncludingClicked,
    ClickedOutsideSelection,
}

impl RemoteRowSelection {
    fn includes_clicked(self) -> bool {
        !matches!(self, Self::ClickedOutsideSelection)
    }

    fn is_single(self) -> bool {
        matches!(self, Self::ClickedOnly)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextSubject {
    Row {
        entry_kind: RemoteContextEntryKind,
        selection: RemoteRowSelection,
    },
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct RemoteContextCapabilities {
    pub(in crate::app) open_with_chooser: bool,
    pub(in crate::app) file_clipboard: bool,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct RemoteContextMenu {
    pub(in crate::app) pos: egui::Pos2,
    pub(in crate::app) target: RemoteContextTarget,
    row_path: Option<Arc<str>>,
}

/// Menu availability is calculated before rendering, making the supported
/// remote operations explicit and deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextAction {
    Open,
    OpenWith,
    DownloadTo,
    CopyToClipboard,
    Rename,
    Delete,
    ToggleFavorite,
    CopyPath,
    AnalyzeDirectory,
    Paste,
    NewFolder,
    NewFile(RemoteEditableFile),
    SelectAll,
    InvertSelection,
    AnalyzeCurrentFolder,
    Refresh,
}

/// The object an action operates on. Keeping this mapping explicit prevents a
/// row menu from silently applying a clicked-row label to the whole selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextActionTarget {
    ClickedRow,
    CurrentSelection,
    CurrentFolder,
    CurrentView,
}

impl RemoteContextAction {
    pub(in crate::app) fn target(self) -> RemoteContextActionTarget {
        match self {
            Self::Open
            | Self::OpenWith
            | Self::DownloadTo
            | Self::ToggleFavorite
            | Self::CopyPath
            | Self::AnalyzeDirectory => RemoteContextActionTarget::ClickedRow,
            Self::CopyToClipboard | Self::Rename | Self::Delete => {
                RemoteContextActionTarget::CurrentSelection
            }
            Self::Paste
            | Self::NewFolder
            | Self::NewFile(_)
            | Self::AnalyzeCurrentFolder
            | Self::Refresh => RemoteContextActionTarget::CurrentFolder,
            Self::SelectAll | Self::InvertSelection => RemoteContextActionTarget::CurrentView,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteEditableFile {
    Text,
    Markdown,
    Csv,
    Json,
    Html,
    Rust,
}

impl RemoteEditableFile {
    fn details(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Text => ("📄 Textdatei (.txt)", "Neue Textdatei", "txt"),
            Self::Markdown => ("📝 Markdown (.md)", "Neue Notiz", "md"),
            Self::Csv => ("📊 CSV (.csv)", "Neue Tabelle", "csv"),
            Self::Json => ("🔧 JSON (.json)", "Neue Datei", "json"),
            Self::Html => ("🌐 HTML (.html)", "Neue Seite", "html"),
            Self::Rust => ("</> Code (.rs)", "Neue Datei", "rs"),
        }
    }
}

const EDITABLE_FILES: [RemoteEditableFile; 6] = [
    RemoteEditableFile::Text,
    RemoteEditableFile::Markdown,
    RemoteEditableFile::Csv,
    RemoteEditableFile::Json,
    RemoteEditableFile::Html,
    RemoteEditableFile::Rust,
];

pub(in crate::app) fn plan_remote_context_menu(
    subject: RemoteContextSubject,
    capabilities: RemoteContextCapabilities,
) -> Vec<RemoteContextAction> {
    match subject {
        RemoteContextSubject::Row {
            entry_kind,
            selection,
        } => {
            let row_is_dir = entry_kind == RemoteContextEntryKind::Directory;
            let mut actions = vec![RemoteContextAction::Open];
            if !row_is_dir && capabilities.open_with_chooser {
                actions.push(RemoteContextAction::OpenWith);
            }
            actions.push(RemoteContextAction::DownloadTo);
            if selection.includes_clicked() && capabilities.file_clipboard {
                actions.push(RemoteContextAction::CopyToClipboard);
            }
            if selection.is_single() {
                actions.push(RemoteContextAction::Rename);
            }
            if selection.includes_clicked() {
                actions.push(RemoteContextAction::Delete);
            }
            if selection.is_single() && row_is_dir {
                actions.push(RemoteContextAction::ToggleFavorite);
            }
            actions.push(RemoteContextAction::CopyPath);
            actions.push(if row_is_dir {
                RemoteContextAction::AnalyzeDirectory
            } else {
                RemoteContextAction::AnalyzeCurrentFolder
            });
            actions.push(RemoteContextAction::Refresh);
            actions
        }
        RemoteContextSubject::Background => {
            let mut actions = Vec::new();
            if capabilities.file_clipboard {
                actions.push(RemoteContextAction::Paste);
            }
            actions.push(RemoteContextAction::NewFolder);
            actions.extend(EDITABLE_FILES.map(RemoteContextAction::NewFile));
            actions.extend([
                RemoteContextAction::SelectAll,
                RemoteContextAction::InvertSelection,
                RemoteContextAction::AnalyzeCurrentFolder,
                RemoteContextAction::Refresh,
            ]);
            actions
        }
    }
}

impl App {
    pub(in crate::app) fn open_remote_context_menu(
        &mut self,
        pos: egui::Pos2,
        target: RemoteContextTarget,
    ) {
        let row_path = match target {
            RemoteContextTarget::Row { entry_idx } => {
                let Some(entry) = self.entries.get(entry_idx) else {
                    self.remote_ctx = None;
                    return;
                };
                Some(entry.path.clone())
            }
            RemoteContextTarget::Background => None,
        };
        self.remote_ctx = Some(RemoteContextMenu {
            pos,
            target,
            row_path,
        });
    }

    pub(in crate::app) fn ui_remote_ctx(&mut self, ctx: &egui::Context) {
        if self.remote.is_none() {
            self.remote_ctx = None;
            return;
        }
        let Some(menu) = self.remote_ctx.clone() else {
            return;
        };
        let row = match menu.target {
            RemoteContextTarget::Row { entry_idx } => {
                let Some(row_path) = menu.row_path.as_ref() else {
                    self.remote_ctx = None;
                    return;
                };
                let original_still_matches = self.entries.get(entry_idx).is_some_and(|entry| {
                    entry.path.as_ref() == row_path.as_ref()
                });
                if original_still_matches {
                    Some(entry_idx)
                } else {
                    let resolved = self
                        .entries
                        .iter()
                        .position(|entry| entry.path.as_ref() == row_path.as_ref());
                    let Some(resolved) = resolved else {
                        self.remote_ctx = None;
                        return;
                    };
                    Some(resolved)
                }
            }
            RemoteContextTarget::Background => None,
        };
        let row_context = row.map(|idx| {
            let entry = &self.entries[idx];
            let entry_kind = if entry.is_dir {
                RemoteContextEntryKind::Directory
            } else {
                RemoteContextEntryKind::File
            };
            let selection = if self.selection.contains(&entry.key()) {
                if self.selection.len() == 1 {
                    RemoteRowSelection::ClickedOnly
                } else {
                    RemoteRowSelection::MultipleIncludingClicked
                }
            } else {
                RemoteRowSelection::ClickedOutsideSelection
            };
            (entry_kind, selection)
        });
        let subject = match row_context {
            Some((entry_kind, selection)) => RemoteContextSubject::Row {
                entry_kind,
                selection,
            },
            None => RemoteContextSubject::Background,
        };
        let actions = plan_remote_context_menu(
            subject,
            RemoteContextCapabilities {
                open_with_chooser: OPEN_WITH_CHOOSER_SUPPORTED,
                file_clipboard: clipboard_file_ops_supported(),
            },
        );
        let planned_files: Vec<RemoteEditableFile> = actions
            .iter()
            .filter_map(|action| match action {
                RemoteContextAction::NewFile(kind) => Some(*kind),
                _ => None,
            })
            .collect();
        let mut selected = None;
        let mut submenu_rect = None;
        let area = egui::Area::new(egui::Id::new("remote_ctx_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu.pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(220.0);
                    for action in actions {
                        if matches!(action, RemoteContextAction::NewFile(_)) {
                            continue;
                        }
                        if action == RemoteContextAction::NewFolder {
                            ui.menu_button("＋ Neu", |ui| {
                                if ui.button("📁 Ordner").clicked() {
                                    selected = Some(RemoteContextAction::NewFolder);
                                    ui.close_menu();
                                }
                                ui.separator();
                                for kind in &planned_files {
                                    let kind = *kind;
                                    let (label, _, _) = kind.details();
                                    if ui.button(label).clicked() {
                                        selected = Some(RemoteContextAction::NewFile(kind));
                                        ui.close_menu();
                                    }
                                }
                                submenu_rect = Some(ui.min_rect());
                            });
                            continue;
                        }
                        let response = ui.button(self.remote_context_action_label(
                            action,
                            row_context,
                            row,
                        ));
                        let clicked = if action == RemoteContextAction::OpenWith {
                            response
                                .on_hover_text(
                                    "Lädt die Datei lokal und öffnet Windows' „Öffnen mit“-Auswahl",
                                )
                                .clicked()
                        } else {
                            response.clicked()
                        };
                        if clicked {
                            selected = Some(action);
                        }
                    }
                });
            });
        let escape_pressed = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let pressed_outside = ctx.input(|input| {
            input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|pointer| {
                    !area.response.rect.contains(pointer)
                        && !submenu_rect.is_some_and(|rect| rect.contains(pointer))
                })
        });
        let dismiss = escape_pressed || pressed_outside;
        let Some(action) = selected else {
            if dismiss {
                self.remote_ctx = None;
            }
            return;
        };
        self.remote_ctx = None;
        self.dispatch_remote_context_action(ctx, row, action);
    }

    fn remote_context_action_label(
        &self,
        action: RemoteContextAction,
        row_context: Option<(RemoteContextEntryKind, RemoteRowSelection)>,
        row: Option<usize>,
    ) -> &'static str {
        match action {
            RemoteContextAction::Open => {
                if row_context
                    .is_some_and(|(kind, _)| kind == RemoteContextEntryKind::Directory)
                {
                    "📂 Öffnen"
                } else {
                    "📄 Öffnen"
                }
            }
            RemoteContextAction::OpenWith => "📂 Öffnen mit…",
            RemoteContextAction::DownloadTo => "⬇ Diesen Eintrag herunterladen nach…",
            RemoteContextAction::CopyToClipboard => {
                if row_context.is_some_and(|(_, selection)| {
                    selection == RemoteRowSelection::MultipleIncludingClicked
                }) {
                    "📋 Auswahl in Zwischenablage kopieren"
                } else {
                    "📋 In Zwischenablage kopieren"
                }
            }
            RemoteContextAction::Rename => "✎ Umbenennen",
            RemoteContextAction::Delete => {
                if row_context.is_some_and(|(_, selection)| {
                    selection == RemoteRowSelection::MultipleIncludingClicked
                }) {
                    "🗑 Auswahl löschen"
                } else {
                    "🗑 Löschen"
                }
            }
            RemoteContextAction::ToggleFavorite => {
                let is_favorite = row.is_some_and(|idx| {
                    let key = self.location_key(self.entries[idx].path.as_ref());
                    self.is_favorite(&key)
                });
                if is_favorite {
                    "☆ Aus Favoriten entfernen"
                } else {
                    "★ Zu Favoriten"
                }
            }
            RemoteContextAction::CopyPath => "⧉ Remote-Pfad kopieren",
            RemoteContextAction::AnalyzeDirectory => "📊 Ordner analysieren",
            RemoteContextAction::Paste => "📥 Einfügen",
            RemoteContextAction::NewFolder | RemoteContextAction::NewFile(_) => unreachable!(),
            RemoteContextAction::SelectAll => "☑ Alles auswählen",
            RemoteContextAction::InvertSelection => "⇄ Auswahl umkehren",
            RemoteContextAction::AnalyzeCurrentFolder => "📊 Aktuellen Ordner analysieren",
            RemoteContextAction::Refresh => "⟳ Aktualisieren",
        }
    }

    fn dispatch_remote_context_action(
        &mut self,
        ctx: &egui::Context,
        row: Option<usize>,
        action: RemoteContextAction,
    ) {
        match action.target() {
            RemoteContextActionTarget::ClickedRow if row.is_none() => return,
            RemoteContextActionTarget::CurrentSelection if self.selection.is_empty() => return,
            _ => {}
        }
        let row_path = row.map(|idx| self.entries[idx].path.to_string());
        match action {
            RemoteContextAction::Open => {
                if let Some(idx) = row {
                    self.activate_entry(idx);
                }
            }
            RemoteContextAction::OpenWith => {
                if let Some(idx) = row {
                    self.open_with_entry(idx);
                }
            }
            RemoteContextAction::DownloadTo => {
                if let Some(src) = row_path {
                    self.open_picker(PickerPurpose::DownloadTo { src }, "");
                }
            }
            RemoteContextAction::CopyToClipboard => self.clipboard_copy_files(false),
            RemoteContextAction::Rename => self.open_rename(),
            RemoteContextAction::Delete => self.trash_selected(),
            RemoteContextAction::ToggleFavorite => {
                if let Some(path) = row_path {
                    let key = self.location_key(&path);
                    self.toggle_favorite(&key);
                }
            }
            RemoteContextAction::CopyPath => {
                if let Some(path) = row_path {
                    ctx.copy_text(path);
                }
            }
            RemoteContextAction::AnalyzeDirectory => {
                if let Some(root) = row_path {
                    self.start_remote_context_analysis(root);
                }
            }
            RemoteContextAction::Paste => self.clipboard_paste_files(),
            RemoteContextAction::NewFolder => self.create_new_folder(),
            RemoteContextAction::NewFile(kind) => {
                let (_, base, ext) = kind.details();
                self.create_new_file(base, ext);
            }
            RemoteContextAction::SelectAll => self.select_all(),
            RemoteContextAction::InvertSelection => self.invert_selection(),
            RemoteContextAction::AnalyzeCurrentFolder => {
                self.start_remote_context_analysis(self.root_path.clone());
            }
            RemoteContextAction::Refresh => self.rescan(),
        }
    }

    fn start_remote_context_analysis(&mut self, root: String) {
        let Some(backend) = self.remote.as_ref().map(|remote| remote.backend.clone()) else {
            return;
        };
        self.show_analytics = true;
        self.start_analytics_scan_remote(backend, root.clone(), root);
    }
}
