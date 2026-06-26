use super::super::prelude::*;
use super::super::*;

impl App {
    pub(in crate::app) fn show_shell_menu_for(&mut self, clicked_path: &str, ctx: &egui::Context) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};

        let clicked_arc: Arc<str> = Arc::from(clicked_path);
        let paths: Vec<String> =
            if self.selection.contains(&clicked_arc) && self.selection.len() > 1 {
                self.selection
                    .iter()
                    .map(|k| sel_key_path(k).replace('/', "\\"))
                    .collect()
            } else {
                vec![clicked_path.replace('/', "\\")]
            };

        let filter_active = self.filter_is_active();
        let own = vec![
            OwnMenuItem {
                id: menu_ids::COPY,
                label: if filter_active {
                    "Kopieren (mit Filter)".to_string()
                } else {
                    "Kopieren".to_string()
                },
            },
            OwnMenuItem {
                id: menu_ids::CUT,
                label: "Ausschneiden".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_PATH,
                label: "Pfad kopieren".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_TO,
                label: "Kopieren nach... (Filter + Struktur)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::MOVE_TO,
                label: "Verschieben nach...".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::RENAME,
                label: "Umbenennen (F2)".to_string(),
            },
        ];

        let clicked_fwd = clicked_path.replace('\\', "/");
        let clicked_is_dir = self
            .entries
            .iter()
            .any(|e| e.is_dir && e.path.as_ref() == clicked_fwd);
        let mut own = own;
        if clicked_is_dir {
            own.push(OwnMenuItem {
                id: menu_ids::TOGGLE_FAV,
                label: if self.is_favorite(&clicked_fwd) {
                    "Aus Favoriten entfernen".to_string()
                } else {
                    "Zu Favoriten".to_string()
                },
            });
        } else if is_zip_name(&clicked_fwd) {
            own.push(OwnMenuItem {
                id: menu_ids::EXTRACT_ZIP,
                label: "Hier entpacken".to_string(),
            });
        }

        match crate::shell_menu::show_for_paths(&paths, None, None, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::COPY => self.clipboard_copy_files(false),
                menu_ids::CUT => self.clipboard_copy_files(true),
                menu_ids::COPY_PATH => self.copy_paths_to_clipboard(ctx),
                menu_ids::COPY_TO => {
                    self.copy_mode_pending = CopyMode::Copy;
                    self.copy_open = true;
                }
                menu_ids::MOVE_TO => {
                    self.copy_mode_pending = CopyMode::Move;
                    self.copy_open = true;
                }
                menu_ids::RENAME => self.open_rename(),
                menu_ids::TOGGLE_FAV => self.toggle_favorite(&clicked_fwd),
                menu_ids::EXTRACT_ZIP => self.start_zip_extract(clicked_fwd.clone()),
                _ => {}
            },
            Ok(MenuResult::Shell) => {
                self.rescan();
            }
            _ => {}
        }
    }

    pub(in crate::app) fn show_background_menu(&mut self) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};
        if self.root_path.is_empty() {
            return;
        }
        let own = vec![
            OwnMenuItem {
                id: menu_ids::PASTE,
                label: "Einfuegen".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::NEW_FOLDER,
                label: "Neuer Ordner (Ctrl+Shift+N)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::SELECT_ALL,
                label: "Alles auswaehlen (Ctrl+A)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::REFRESH,
                label: "Aktualisieren (F5)".to_string(),
            },
        ];
        let folder = self.root_path.replace('/', "\\");
        match crate::shell_menu::show_background_menu(&folder, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::PASTE => self.clipboard_paste_files(),
                menu_ids::NEW_FOLDER => self.create_new_folder(),
                menu_ids::SELECT_ALL => self.select_all(),
                menu_ids::REFRESH => self.rescan(),
                _ => {}
            },
            Ok(MenuResult::Shell) => self.rescan(),
            _ => {}
        }
    }
}
