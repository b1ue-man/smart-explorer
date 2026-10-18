//! Context menus of the surface and the handlings behind them (DSK-M4,
//! C4.7): device (Öffnen, Im Explorer/Dateimanager öffnen, Bildschirm
//! ansehen, Bildschirm steuern, Geräteseite), room (Mitglieder, Bildschirme
//! des Raums, Raumseite), vault and the other tiles (Öffnen), desktop file
//! (Öffnen, Öffnen mit, Umbenennen, In Papierkorb, Ausschneiden, Kopieren),
//! every tile (Auf Bildschirm verschieben) and the surface (Neuer Ordner,
//! Einfügen, Anordnen, Aktualisieren, Hintergrund ändern, se Desktop-
//! Einstellungen, se Desktop neu starten). Double click and Enter run
//! [`activate`], the keys of the grid the same handlings as the menus.
use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui;

use super::desktop_surface_actions::{DesktopOp, SurfaceActions};
use super::desktop_surface_grid_ui::SurfaceState;
use super::desktop_surface_host::{self as host_ui, SurfaceHost};
use super::desktop_surface_rename_ui::Rename;
use super::desktop_surface_view_model::{ItemRef, ViewModel};
use crate::desktop::core::items::{self, DesktopFile};
use crate::desktop::core::layout::{ItemId, LayoutOp};
use crate::desktop::core::state::Platform;
use crate::desktop::core::tiles::{DeviceTile, RoomGroup};
use crate::desktop::os::open_target::{self, OpenRequest};
use crate::desktop::os::platform;

/// Everything a handling of the surface reaches.
pub(crate) struct Cx<'a> {
    pub(crate) view: &'a mut ViewModel,
    pub(crate) state: &'a mut SurfaceState,
    pub(crate) host: &'a dyn SurfaceHost,
    pub(crate) actions: &'a mut dyn SurfaceActions,
}

/// A selected desktop file.
struct Picked {
    item: ItemId,
    path: PathBuf,
    name: String,
    public: bool,
}

fn file_manager_label() -> &'static str {
    if platform::PLATFORM == Platform::Windows {
        "Im Explorer öffnen"
    } else {
        "Im Dateimanager öffnen"
    }
}

/// Double click and Enter: open the item; a room shows or hides its members.
pub(crate) fn activate(item: &ItemId, cx: &mut Cx<'_>) {
    let with = cx.view.config().open_devices_with;
    let request = match cx.view.lookup(item) {
        Some(ItemRef::Device(tile)) => open_target::open_device(tile, with),
        Some(ItemRef::Room(room)) => {
            let open = cx.state.open_room.as_deref() == Some(room.room_id.as_str());
            cx.state.open_room = (!open).then(|| room.room_id.clone());
            return;
        }
        Some(ItemRef::Vault(vault)) => open_target::open_vault(vault),
        Some(ItemRef::File(file)) => match cx.view.folder(file.public) {
            Some(folder) => open_target::desktop_file(folder, &file.name, false),
            None => return,
        },
        Some(ItemRef::System(system)) => open_target::system_item(system),
        Some(ItemRef::AllScreens) => open_target::all_screens(),
        Some(ItemRef::Empty(action)) => open_target::empty_action(action),
        None => return,
    };
    cx.actions.open(request);
}

/// The context menu of one item.
pub(crate) fn item_menu(ui: &mut egui::Ui, item: &ItemId, cx: &mut Cx<'_>) {
    match cx.view.lookup(item) {
        Some(ItemRef::Device(tile)) => {
            let tile = tile.clone();
            device_menu(ui, &tile, cx);
        }
        Some(ItemRef::Room(room)) => {
            let room = room.clone();
            room_menu(ui, &room, cx);
        }
        Some(ItemRef::File(file)) => {
            let file = file.clone();
            file_menu(ui, item, &file, cx);
        }
        Some(_) => {
            if ui.button("Öffnen").clicked() {
                activate(item, cx);
                ui.close_menu();
            }
        }
        None => {
            ui.close_menu();
            return;
        }
    }
    move_menu(ui, item, cx);
}

fn device_menu(ui: &mut egui::Ui, tile: &DeviceTile, cx: &mut Cx<'_>) {
    let with = cx.view.config().open_devices_with;
    let mut chosen = None;
    if ui.button("Öffnen").clicked() {
        chosen = Some(open_target::open_device(tile, with));
    }
    if let Some(folder) = open_target::device_folder(tile) {
        if ui.button(file_manager_label()).clicked() {
            chosen = Some(folder);
        }
    }
    for (label, control) in [("Bildschirm ansehen", false), ("Bildschirm steuern", true)] {
        if let Some(screen) = open_target::device_screen(tile, control) {
            if ui.button(label).clicked() {
                chosen = Some(screen);
            }
        }
    }
    if ui.button("Geräteseite").clicked() {
        chosen = Some(open_target::device_page(tile));
    }
    if let Some(request) = chosen {
        cx.actions.open(request);
        ui.close_menu();
    }
}

fn room_menu(ui: &mut egui::Ui, room: &RoomGroup, cx: &mut Cx<'_>) {
    if ui.button("Mitglieder").clicked() {
        cx.state.open_room = Some(room.room_id.clone());
        ui.close_menu();
    }
    if ui.button("Bildschirme des Raums").clicked() {
        cx.actions.open(open_target::room_screens(room));
        ui.close_menu();
    }
    if ui.button("Raumseite").clicked() {
        cx.actions.open(open_target::room_page(room));
        ui.close_menu();
    }
}

fn file_menu(ui: &mut egui::Ui, item: &ItemId, file: &DesktopFile, cx: &mut Cx<'_>) {
    let Some(folder) = cx
        .view
        .folder(file.public)
        .map(|folder| folder.to_path_buf())
    else {
        ui.label("Desktop-Ordner unbekannt");
        return;
    };
    if !cx.state.selected.contains(item) {
        cx.state.selected = BTreeSet::from([item.clone()]);
    }
    for (label, choose) in [("Öffnen", false), ("Öffnen mit …", true)] {
        if ui.button(label).clicked() {
            cx.actions
                .open(open_target::desktop_file(&folder, &file.name, choose));
            ui.close_menu();
        }
    }
    if ui.button("Umbenennen").clicked() {
        start_rename(item, cx);
        ui.close_menu();
    }
    if ui.button("In Papierkorb").clicked() {
        recycle_selected(cx);
        ui.close_menu();
    }
    for (label, cut) in [("Ausschneiden", true), ("Kopieren", false)] {
        if ui.button(label).clicked() {
            clipboard_selected(ui.ctx(), cut, cx);
            ui.close_menu();
        }
    }
}

/// "Auf Bildschirm verschieben → <Monitor>" (C4.2).
fn move_menu(ui: &mut egui::Ui, item: &ItemId, cx: &mut Cx<'_>) {
    let others = host_ui::other_monitors(cx.host);
    if others.is_empty() {
        return;
    }
    ui.menu_button("Auf Bildschirm verschieben", |ui| {
        for (monitor, name) in others {
            if ui.button(name).clicked() {
                let item = item.clone();
                cx.view.layout_op(LayoutOp::MoveToMonitor { item, monitor });
                ui.close_menu();
            }
        }
    });
}

/// The context menu of the free surface.
pub(crate) fn surface_menu(ui: &mut egui::Ui, cx: &mut Cx<'_>) {
    let folder = cx.view.folder(false).map(|folder| folder.to_path_buf());
    let has_folder = folder.is_some();
    if ui
        .add_enabled(has_folder, egui::Button::new("Neuer Ordner"))
        .clicked()
    {
        if let Some(folder) = folder {
            cx.actions.run(DesktopOp::NewFolder { folder });
        }
        ui.close_menu();
    }
    if ui
        .add_enabled(has_folder, egui::Button::new("Einfügen"))
        .clicked()
    {
        paste(None, cx);
        ui.close_menu();
    }
    if ui.button("Anordnen").clicked() {
        arrange(cx);
        ui.close_menu();
    }
    if ui.button("Aktualisieren").clicked() {
        cx.state.refresh(cx.view);
        ui.close_menu();
    }
    ui.separator();
    for label in ["Hintergrund ändern …", "se Desktop-Einstellungen …"] {
        if ui.button(label).clicked() {
            cx.actions.open(open_target::settings_page());
            ui.close_menu();
        }
    }
    if ui.button("se Desktop neu starten").clicked() {
        cx.actions.open(OpenRequest::Restart);
        ui.close_menu();
    }
}

/// "Anordnen": the order in which new items are placed (C1.2).
pub(crate) fn arrange(cx: &mut Cx<'_>) {
    let Some(model) = cx.view.model() else {
        return;
    };
    let order = items::surface_items(
        platform::PLATFORM,
        cx.view.config(),
        &model.files,
        &model.tiles,
    );
    let monitor = cx.view.monitor().to_string();
    cx.view.layout_op(LayoutOp::Arrange { monitor, order });
}

/// F2 and "Umbenennen": desktop files only.
pub(crate) fn start_rename(item: &ItemId, cx: &mut Cx<'_>) {
    if let Some(ItemRef::File(file)) = cx.view.lookup(item) {
        cx.state.rename = Some(Rename::new(item.clone(), &file.name));
    }
}

fn selected_files(cx: &Cx<'_>) -> Vec<Picked> {
    cx.state
        .selected
        .iter()
        .filter_map(|item| match cx.view.lookup(item) {
            Some(ItemRef::File(file)) => Some(Picked {
                item: item.clone(),
                path: cx.view.file_path(file)?,
                name: file.name.clone(),
                public: file.public,
            }),
            _ => None,
        })
        .collect()
}

/// Del and "In Papierkorb": the selected desktop files.
pub(crate) fn recycle_selected(cx: &mut Cx<'_>) {
    let picked = selected_files(cx);
    for public in [false, true] {
        let names: Vec<String> = picked
            .iter()
            .filter(|file| file.public == public)
            .map(|file| file.name.clone())
            .collect();
        let Some(folder) = cx.view.folder(public).map(|folder| folder.to_path_buf()) else {
            continue;
        };
        if !names.is_empty() {
            cx.actions.run(DesktopOp::Recycle { folder, names });
        }
    }
    cx.state.selected.clear();
}

/// Ctrl+C, Ctrl+X and the menu entries: the selected desktop files.
pub(crate) fn clipboard_selected(ctx: &egui::Context, cut: bool, cx: &mut Cx<'_>) {
    let picked = selected_files(cx);
    if picked.is_empty() {
        return;
    }
    cx.state.cut = if cut {
        picked.iter().map(|file| file.item.clone()).collect()
    } else {
        BTreeSet::new()
    };
    let paths = picked.into_iter().map(|file| file.path).collect();
    cx.actions.set_clipboard(ctx, paths, cut);
}

/// Ctrl+V and "Einfügen": into the user's desktop folder.
pub(crate) fn paste(pasted: Option<&str>, cx: &mut Cx<'_>) {
    let Some(folder) = cx.view.folder(false).map(|folder| folder.to_path_buf()) else {
        return;
    };
    if let Some((sources, cut)) = cx.actions.clipboard(pasted) {
        cx.state.cut.clear();
        cx.actions.run(DesktopOp::Transfer {
            sources,
            folder,
            cut,
        });
    }
}
