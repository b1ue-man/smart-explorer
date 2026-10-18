//! Keys and dropped files of the surface (DSK-M4, C4.7): arrows move the
//! selection to the nearest tile, Enter opens, F2 renames, Del moves to the
//! recycle bin, Esc clears, Ctrl+A selects all, Ctrl+C/X/V copy, cut and
//! paste desktop files, F5 refreshes; files dropped from programs go into
//! the desktop folder (Windows, X11; Wayland: `DESKTOP-WAYLAND-DND`). egui
//! turns Ctrl+C/X/V into its Copy, Cut and Paste events; a paste reaches the
//! surface only while the clipboard holds text (the surface's own copies put
//! their paths there outside Windows).
use std::path::{Path, PathBuf};

use eframe::egui;

use super::desktop_surface_actions::DesktopOp;
use super::desktop_surface_grid_ui::select;
use super::desktop_surface_menu_ui::{self as menu, Cx};
use crate::desktop::core::layout::{Cell, ItemId};

#[derive(Clone, Copy)]
enum Step {
    Left,
    Right,
    Up,
    Down,
}

/// The nearest item in the direction of `step` from `from`.
fn step_target(items: &[(ItemId, Cell)], from: Cell, step: Step) -> Option<&ItemId> {
    let delta = |to: Cell| {
        (
            i64::from(to.col) - i64::from(from.col),
            i64::from(to.row) - i64::from(from.row),
        )
    };
    items
        .iter()
        .filter(|(_, cell)| {
            let (dx, dy) = delta(*cell);
            match step {
                Step::Left => dx < 0,
                Step::Right => dx > 0,
                Step::Up => dy < 0 && dx == 0,
                Step::Down => dy > 0 && dx == 0,
            }
        })
        .min_by_key(|(_, cell)| {
            let (dx, dy) = delta(*cell);
            match step {
                Step::Left | Step::Right => (dy.abs(), dx.abs()),
                Step::Up | Step::Down => (dx.abs(), dy.abs()),
            }
        })
        .map(|(item, _)| item)
}

/// The keys of one frame; `items` are the shown tiles in reading order.
pub(crate) fn keyboard(ctx: &egui::Context, items: &[(ItemId, Cell)], cx: &mut Cx<'_>) {
    if cx.state.rename.is_some() {
        return;
    }
    let events = ctx.input(|input| input.events.clone());
    for event in &events {
        match event {
            egui::Event::Copy => menu::clipboard_selected(ctx, false, cx),
            egui::Event::Cut => menu::clipboard_selected(ctx, true, cx),
            egui::Event::Paste(text) => menu::paste(Some(text.as_str()), cx),
            _ => {}
        }
    }
    let pressed = |key: egui::Key| ctx.input(|input| input.key_pressed(key));
    if ctx.input(|input| input.modifiers.command) && pressed(egui::Key::A) {
        cx.state.selected = items.iter().map(|(item, _)| item.clone()).collect();
    }
    if pressed(egui::Key::Escape) {
        cx.state.selected.clear();
        cx.state.open_room = None;
    }
    if pressed(egui::Key::F5) {
        cx.state.refresh(cx.view);
    }
    if pressed(egui::Key::Delete) {
        menu::recycle_selected(cx);
    }
    let current = cx
        .state
        .focus
        .clone()
        .or_else(|| cx.state.selected.iter().next().cloned());
    if pressed(egui::Key::F2) {
        if let Some(item) = &current {
            menu::start_rename(item, cx);
        }
    }
    if pressed(egui::Key::Enter) {
        if let Some(item) = &current {
            menu::activate(item, cx);
        }
    }
    let steps = [
        (egui::Key::ArrowLeft, Step::Left),
        (egui::Key::ArrowRight, Step::Right),
        (egui::Key::ArrowUp, Step::Up),
        (egui::Key::ArrowDown, Step::Down),
    ];
    for (key, step) in steps {
        if !pressed(key) {
            continue;
        }
        let from = current
            .as_ref()
            .and_then(|focus| items.iter().find(|(item, _)| item == focus))
            .map(|(_, cell)| *cell);
        let target = match from {
            Some(from) => step_target(items, from, step),
            None => items.first().map(|(item, _)| item),
        };
        if let Some(target) = target.cloned() {
            select(cx.state, &target, false);
        }
    }
}

/// Files dropped from programs go into the user's desktop folder; with
/// Shift they move.
pub(crate) fn dropped_files(ctx: &egui::Context, cx: &mut Cx<'_>) {
    let (sources, cut) = ctx.input(|input| {
        let sources: Vec<PathBuf> = input
            .raw
            .dropped_files
            .iter()
            .filter_map(|file| file.path.clone())
            .collect();
        (sources, input.modifiers.shift)
    });
    if sources.is_empty() {
        return;
    }
    match cx.view.folder(false).map(Path::to_path_buf) {
        Some(folder) => {
            // A file of the desktop folder dropped on its own surface (a
            // drag out that came back) stays where it is.
            let sources: Vec<PathBuf> = sources
                .into_iter()
                .filter(|source| source.parent() != Some(folder.as_path()))
                .collect();
            if !sources.is_empty() {
                cx.actions.run(DesktopOp::Transfer {
                    sources,
                    folder,
                    cut,
                });
            }
        }
        None => cx
            .state
            .notice("Kein Desktop-Ordner bekannt: Ablegen nicht möglich".to_string()),
    }
}
