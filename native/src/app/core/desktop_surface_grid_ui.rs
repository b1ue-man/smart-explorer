//! The surface view (DSK-M4, C4.2, C4.6, C4.7), written against
//! `egui::Context` only, so the eframe window and the layer backend of
//! DSK-M7 drive it alike: wallpaper, the grid of this monitor's arrangement
//! with every tile, selection by click and frame, dragging to arrange (over
//! the window's edge onto another monitor, Annahme A31), rooms opening as a
//! group of their member devices, the keys (arrows, Enter, F2, Del, Esc,
//! Ctrl+A/C/X/V, F5), files dropped from programs into the desktop folder
//! and the status panel with "Verbinden".
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::desktop_surface_actions::{DesktopOp, SurfaceActions};
use super::desktop_surface_host::{self as host_ui, SurfaceHost};
use super::desktop_surface_input_ui as input;
use super::desktop_surface_menu_ui::{self as menu, Cx};
use super::desktop_surface_rename_ui::{Rename, RenameOutcome};
use super::desktop_surface_tile_ui::{self as tile_ui, TileState, CELL};
use super::desktop_surface_view_model::{ItemRef, ViewModel};
use super::desktop_surface_wallpaper_ui::Wallpaper;
use crate::desktop::core::layout::{Cell, ItemId, LayoutOp};
use crate::desktop::os::open_target;
use crate::desktop::os::store::unix_now;

/// Space between the work area's edge and the first cell.
const MARGIN: f32 = 12.0;
/// How long a message of a finished handling stays.
const NOTICE_FOR: Duration = Duration::from_secs(8);
const FLYOUT_COLUMNS: f32 = 4.0;

/// What the view keeps between frames.
#[derive(Default)]
pub(crate) struct SurfaceState {
    pub(crate) selected: BTreeSet<ItemId>,
    pub(crate) focus: Option<ItemId>,
    /// The room whose member devices are shown.
    pub(crate) open_room: Option<String>,
    pub(crate) rename: Option<Rename>,
    /// Cut with Ctrl+X, shown dimmed until pasted.
    pub(crate) cut: BTreeSet<ItemId>,
    /// The session gives the surface an alpha channel (`transparent`).
    pub(crate) transparent: bool,
    band: Option<Pos2>,
    sent_rows: Option<u32>,
    notices: Vec<(String, Instant)>,
    wallpaper: Wallpaper,
}

impl SurfaceState {
    /// `transparent`: the session gives the surface an alpha channel.
    pub(crate) fn new(transparent: bool) -> SurfaceState {
        SurfaceState {
            transparent,
            ..SurfaceState::default()
        }
    }

    /// "Aktualisieren" and F5: the wallpaper is read again, the shell sends
    /// its model anew.
    pub(crate) fn refresh(&mut self, view: &ViewModel) {
        self.wallpaper.reload();
        view.refresh();
    }

    /// A message for the status panel.
    pub(crate) fn notice(&mut self, text: String) {
        self.notices.push((text, Instant::now()));
    }
}

/// One frame of the surface.
pub(crate) fn show_surface(
    ctx: &egui::Context,
    view: &mut ViewModel,
    state: &mut SurfaceState,
    host: &dyn SurfaceHost,
    actions: &mut dyn SurfaceActions,
) {
    let now = Instant::now();
    for notice in actions.take_notices() {
        state.notices.push((notice, now));
    }
    state
        .notices
        .retain(|(_, at)| now.saturating_duration_since(*at) < NOTICE_FOR);
    egui::CentralPanel::default()
        .frame(egui::Frame::none())
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            let transparent = state.transparent;
            state
                .wallpaper
                .paint(ui, view.config(), transparent, screen, host);
            let area = work_area(ui.ctx(), screen, host);
            let mut cx = Cx {
                view,
                state,
                host,
                actions,
            };
            grid(ui, area, &mut cx);
            status_panel(ui.ctx(), area, &mut cx);
        });
}

/// The work area in points of the window (the whole window while unknown).
fn work_area(ctx: &egui::Context, screen: Rect, host: &dyn SurfaceHost) -> Rect {
    let (Some(monitor), Some(work)) = (host_ui::monitor_rect(host), host.work_area()) else {
        return screen;
    };
    let ppp = ctx.pixels_per_point();
    let offset = |x: i32, y: i32| screen.min + Vec2::new(x as f32, y as f32) / ppp;
    let area = Rect::from_min_max(
        offset(work[0] - monitor[0], work[1] - monitor[1]),
        offset(work[2] - monitor[0], work[3] - monitor[1]),
    );
    area.intersect(screen)
}

/// The rectangle of a grid cell in the work area.
pub(crate) fn cell_rect(area: Rect, cell: Cell) -> Rect {
    let x = area.left() + MARGIN + cell.col as f32 * CELL.x;
    let y = area.top() + MARGIN + cell.row as f32 * CELL.y;
    Rect::from_min_size(Pos2::new(x, y), CELL)
}

fn cell_at(area: Rect, pos: Pos2, rows: u32) -> Cell {
    let col = ((pos.x - area.left() - MARGIN) / CELL.x).floor().max(0.0) as u32;
    let row = ((pos.y - area.top() - MARGIN) / CELL.y).floor().max(0.0) as u32;
    Cell {
        col,
        row: row.min(rows.saturating_sub(1)),
    }
}

/// Rows of whole cells in the work area (the shell's `Grid` value).
pub(crate) fn rows_for(area: Rect) -> u32 {
    (((area.height() - 2.0 * MARGIN) / CELL.y).floor() as u32).max(1)
}

fn grid(ui: &mut egui::Ui, area: Rect, cx: &mut Cx<'_>) {
    let rows = rows_for(area);
    if cx.view.rows().is_some_and(|current| current != rows) && cx.state.sent_rows != Some(rows) {
        cx.state.sent_rows = Some(rows);
        let monitor = cx.view.monitor().to_string();
        cx.view.layout_op(LayoutOp::Grid { monitor, rows });
    }
    background(ui, area, cx);
    let items = cx.view.visible();
    let now = unix_now();
    let placed: Vec<(ItemId, Rect)> = items
        .iter()
        .map(|(item, cell)| (item.clone(), cell_rect(area, *cell)))
        .collect();
    for (item, rect) in &placed {
        item_tile(ui, item, *rect, area, now, cx);
    }
    if let Some(room) = cx.state.open_room.clone() {
        room_flyout(ui.ctx(), &room, &placed, now, cx);
    }
    rename_editor(ui, &placed, cx);
    rubber_band(ui, &placed, cx);
    input::keyboard(ui.ctx(), &items, cx);
    input::dropped_files(ui.ctx(), cx);
}

fn background(ui: &mut egui::Ui, area: Rect, cx: &mut Cx<'_>) {
    let response = ui.interact(
        area,
        egui::Id::new("se-desktop-surface"),
        Sense::click_and_drag(),
    );
    if response.clicked() || response.secondary_clicked() {
        cx.state.selected.clear();
        cx.state.focus = None;
        cx.state.open_room = None;
    }
    if response.drag_started() {
        let origin = ui.input(|input| input.pointer.press_origin());
        cx.state.band = origin.or(response.interact_pointer_pos());
    }
    response.context_menu(|ui| menu::surface_menu(ui, cx));
}

fn item_tile(ui: &mut egui::Ui, item: &ItemId, rect: Rect, area: Rect, now: i64, cx: &mut Cx<'_>) {
    let Some(found) = cx.view.lookup(item) else {
        return;
    };
    let look = tile_ui::look(found, now);
    let is_room = matches!(found, ItemRef::Room(_));
    let is_file = matches!(found, ItemRef::File(_));
    let id = egui::Id::new(("se-desktop-item", item.encode()));
    let mut response = ui.interact(rect, id, Sense::click_and_drag());
    let state = TileState {
        selected: cx.state.selected.contains(item),
        hovered: response.hovered(),
        dimmed: cx.state.cut.contains(item),
        renaming: cx
            .state
            .rename
            .as_ref()
            .is_some_and(|rename| rename.item == *item),
    };
    tile_ui::paint(ui, rect, &look, state);
    if let Some(tooltip) = &look.tooltip {
        response = response.on_hover_text(tooltip.as_str());
    }
    if response.clicked() {
        let toggle = ui.input(|input| input.modifiers.command);
        select(cx.state, item, toggle);
        if is_room && !toggle {
            menu::activate(item, cx);
        }
    }
    if response.double_clicked() && !is_room {
        menu::activate(item, cx);
    }
    if response.secondary_clicked() && !cx.state.selected.contains(item) {
        select(cx.state, item, false);
    }
    if response.dragged() {
        // DSK-M5: over another program's window the selection goes out
        // through the system's drag and drop (Windows).
        if is_file && menu::drag_out_selected(ui.ctx(), item, cx) {
            return;
        }
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let ghost = Rect::from_center_size(pointer, CELL);
            let ghost_state = TileState {
                dimmed: true,
                ..TileState::default()
            };
            tile_ui::paint(ui, ghost, &look, ghost_state);
        }
    }
    if response.drag_stopped() {
        if let Some(pointer) = response.interact_pointer_pos() {
            drop_item(ui.ctx(), item, pointer, area, cx);
        }
    }
    response.context_menu(|ui| menu::item_menu(ui, item, cx));
}

/// Selects `item` (with `toggle` in addition to the selection) and keeps the
/// keyboard's place there.
pub(crate) fn select(state: &mut SurfaceState, item: &ItemId, toggle: bool) {
    if toggle {
        if !state.selected.remove(item) {
            state.selected.insert(item.clone());
        }
    } else {
        state.selected = BTreeSet::from([item.clone()]);
    }
    state.focus = Some(item.clone());
}

/// A tile let go: its cell here, or the monitor under the pointer when it
/// left the window (Windows, X11, layer backend).
fn drop_item(ctx: &egui::Context, item: &ItemId, pointer: Pos2, area: Rect, cx: &mut Cx<'_>) {
    if !ctx.screen_rect().contains(pointer) {
        // The host knows the point itself (layer backend, logical
        // coordinates); else it follows from the window's rectangle.
        let point = match cx.host.global_point([pointer.x, pointer.y]) {
            Some(point) => point,
            None => {
                let Some(inner) = ctx.input(|input| input.viewport().inner_rect) else {
                    return;
                };
                let global = (inner.min.to_vec2() + pointer.to_vec2()) * ctx.pixels_per_point();
                [global.x.round() as i32, global.y.round() as i32]
            }
        };
        if let Some(monitor) = host_ui::monitor_at(cx.host, point) {
            if monitor != cx.view.monitor() {
                let item = item.clone();
                cx.view.layout_op(LayoutOp::MoveToMonitor { item, monitor });
            }
        }
        return;
    }
    let cell = cell_at(area, pointer, cx.view.rows().unwrap_or(1));
    let monitor = cx.view.monitor().to_string();
    cx.view.layout_op(LayoutOp::Move {
        item: item.clone(),
        monitor,
        col: cell.col,
        row: cell.row,
    });
}

/// The member devices of an open room beside its tile (C4.7).
fn room_flyout(
    ctx: &egui::Context,
    room_id: &str,
    placed: &[(ItemId, Rect)],
    now: i64,
    cx: &mut Cx<'_>,
) {
    let item = ItemId::Room {
        room_id: room_id.to_string(),
    };
    let anchor = placed
        .iter()
        .find(|(placed_item, _)| *placed_item == item)
        .map(|(_, rect)| *rect);
    let (Some(anchor), Some(ItemRef::Room(room))) = (anchor, cx.view.lookup(&item)) else {
        cx.state.open_room = None;
        return;
    };
    let room = room.clone();
    let shown = egui::Area::new(egui::Id::new("se-desktop-room"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.right_top() + Vec2::new(6.0, 0.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(&room.name).strong());
                if room.members.is_empty() {
                    ui.label("Keine Geräte in diesem Raum");
                }
                ui.set_max_width(CELL.x * FLYOUT_COLUMNS);
                ui.horizontal_wrapped(|ui| {
                    for member in &room.members {
                        let member_item = ItemId::Device {
                            key: member.key.clone(),
                        };
                        let (rect, response) = ui.allocate_exact_size(CELL, Sense::click());
                        let look = tile_ui::device_look(member, now);
                        let state = TileState {
                            selected: cx.state.selected.contains(&member_item),
                            hovered: response.hovered(),
                            ..TileState::default()
                        };
                        tile_ui::paint(ui, rect, &look, state);
                        if response.clicked() {
                            select(cx.state, &member_item, false);
                        }
                        if response.double_clicked() {
                            menu::activate(&member_item, cx);
                        }
                        response.context_menu(|ui| menu::item_menu(ui, &member_item, cx));
                    }
                });
            });
        });
    let outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input.pointer.interact_pos().is_some_and(|pointer| {
                !shown.response.rect.contains(pointer) && !anchor.contains(pointer)
            })
    });
    if outside && !ctx.memory(|memory| memory.any_popup_open()) {
        cx.state.open_room = None;
    }
}

fn rename_editor(ui: &mut egui::Ui, placed: &[(ItemId, Rect)], cx: &mut Cx<'_>) {
    let Some(rename) = cx.state.rename.as_mut() else {
        return;
    };
    let Some(rect) = placed
        .iter()
        .find(|(item, _)| *item == rename.item)
        .map(|(_, rect)| *rect)
    else {
        cx.state.rename = None;
        return;
    };
    let public = matches!(rename.item, ItemId::PublicFile { .. });
    match rename.show(ui, rect) {
        RenameOutcome::Editing => {}
        RenameOutcome::Cancel => cx.state.rename = None,
        RenameOutcome::Commit { from, to } => {
            cx.state.rename = None;
            if let Some(folder) = cx.view.folder(public) {
                cx.actions.run(DesktopOp::Rename {
                    folder: folder.to_path_buf(),
                    name: from,
                    new_name: to,
                });
            }
        }
    }
}

/// The selection frame drawn from the free surface.
fn rubber_band(ui: &egui::Ui, placed: &[(ItemId, Rect)], cx: &mut Cx<'_>) {
    let Some(start) = cx.state.band else {
        return;
    };
    let (down, pointer) =
        ui.input(|input| (input.pointer.primary_down(), input.pointer.latest_pos()));
    if let Some(pointer) = pointer {
        let band = Rect::from_two_pos(start, pointer);
        let painter = ui.painter();
        painter.rect_filled(band, 0.0, Color32::from_rgba_unmultiplied(90, 150, 230, 50));
        painter.rect_stroke(
            band,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(120, 170, 235)),
        );
        cx.state.selected = placed
            .iter()
            .filter(|(_, rect)| rect.intersects(band))
            .map(|(item, _)| item.clone())
            .collect();
    }
    if !down {
        cx.state.band = None;
    }
}

/// Connection state, "Verbinden", unreadable profiles, vaults without data
/// and the messages of finished handlings, at the work area's bottom right.
fn status_panel(ctx: &egui::Context, area: Rect, cx: &mut Cx<'_>) {
    let lines = cx.view.status_lines();
    let offers_connect = cx.view.offers_connect();
    let profiles_broken = cx
        .view
        .model()
        .is_some_and(|model| model.tiles.profiles_error.is_some());
    if lines.is_empty() && cx.state.notices.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("se-desktop-status"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(area.right_bottom() - Vec2::splat(MARGIN))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                for line in &lines {
                    ui.label(line.as_str());
                }
                if offers_connect && ui.button("Verbinden").clicked() {
                    cx.view.connect();
                }
                if profiles_broken && ui.button("Geräte & Freigaben öffnen").clicked() {
                    cx.actions.open(open_target::share_page());
                }
                for (notice, _) in &cx.state.notices {
                    ui.colored_label(Color32::from_rgb(255, 154, 158), notice.as_str());
                }
            });
        });
}
