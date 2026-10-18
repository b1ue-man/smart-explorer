//! Tiles of the surface (C4.5, C4.6): one look per device state, rooms as
//! folder groups, vaults with their lock state, desktop files, "Papierkorb"
//! and "Dieser PC", "Alle Bildschirme" and the tiles of the empty state.
//! Labels stay readable on every wallpaper: light text on a translucent dark
//! plate; the state shows as a colored mark at the symbol.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};

use super::desktop_surface_view_model::ItemRef;
use crate::desktop::core::items::SystemItem;
use crate::desktop::core::tiles::{DeviceTile, EmptyAction, TileCode, UNKNOWN_TEXT};

/// Size of one grid cell in points.
pub(crate) const CELL: Vec2 = Vec2::new(104.0, 108.0);
const ICON_FONT: f32 = 36.0;
const TITLE_FONT: f32 = 13.0;
const STATUS_FONT: f32 = 11.0;
const GREEN: Color32 = Color32::from_rgb(76, 175, 80);
const YELLOW: Color32 = Color32::from_rgb(255, 193, 7);
const RED: Color32 = Color32::from_rgb(229, 57, 53);
const GREY: Color32 = Color32::from_rgb(158, 158, 158);
const PLATE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 150);

/// How a mark shows the state at the symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    None,
    Dot(Color32),
    /// "Erreichbar": an outline only.
    Ring(Color32),
    /// "Verbindet": a progress spinner.
    Busy,
}

/// The look of one tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TileLook {
    pub(crate) icon: &'static str,
    pub(crate) title: String,
    pub(crate) status: Option<String>,
    pub(crate) mark: Mark,
    /// Shown on hover (the error of `failed`).
    pub(crate) tooltip: Option<String>,
}

/// "vor 5 Min." and the like for "zuletzt gesehen".
pub(crate) fn relative_age(seconds: i64) -> String {
    match seconds.max(0) {
        0..=59 => "gerade eben".to_string(),
        seconds @ 60..=3_599 => format!("vor {} Min.", seconds / 60),
        seconds @ 3_600..=86_399 => format!("vor {} Std.", seconds / 3_600),
        seconds if seconds < 172_800 => "vor 1 Tag".to_string(),
        seconds => format!("vor {} Tagen", seconds / 86_400),
    }
}

/// C4.5: text and mark of every device state.
pub(crate) fn device_look(tile: &DeviceTile, now_secs: i64) -> TileLook {
    let (status, mark) = match tile.code {
        TileCode::Unknown => (UNKNOWN_TEXT.to_string(), Mark::None),
        TileCode::Offline => {
            let status = match tile.last_seen {
                Some(seen) => format!("Offline, zuletzt gesehen {}", relative_age(now_secs - seen)),
                None => "Offline".to_string(),
            };
            (status, Mark::Dot(GREY))
        }
        TileCode::Waiting => ("Wartet auf das Gerät".to_string(), Mark::Dot(GREY)),
        TileCode::WaitingForAccess => ("Warte auf Freigabe".to_string(), Mark::Dot(YELLOW)),
        TileCode::Available => {
            let status = if tile.lan {
                "Erreichbar im LAN"
            } else {
                "Erreichbar"
            };
            (status.to_string(), Mark::Ring(GREEN))
        }
        TileCode::Connecting => ("Verbindet".to_string(), Mark::Busy),
        TileCode::Connected => ("Verbunden".to_string(), Mark::Dot(GREEN)),
        TileCode::ConnectedDirect => ("Verbunden (direkt)".to_string(), Mark::Dot(GREEN)),
        TileCode::ConnectedRelay => ("Verbunden (Relay)".to_string(), Mark::Dot(GREEN)),
        TileCode::Failed => ("Fehler".to_string(), Mark::Dot(RED)),
        TileCode::IdentityConflict => ("Identität geändert – prüfen".to_string(), Mark::Dot(RED)),
    };
    let tooltip = (tile.code == TileCode::Failed).then(|| tile.status_text.clone());
    TileLook {
        icon: "💻",
        title: tile.name.clone(),
        status: Some(status),
        mark,
        tooltip,
    }
}

fn plain(icon: &'static str, title: &str, status: Option<String>) -> TileLook {
    TileLook {
        icon,
        title: title.to_string(),
        status,
        mark: Mark::None,
        tooltip: None,
    }
}

/// The look of any item.
pub(crate) fn look(item: ItemRef<'_>, now_secs: i64) -> TileLook {
    match item {
        ItemRef::Device(tile) => device_look(tile, now_secs),
        ItemRef::Room(room) => {
            let count = match room.members.len() {
                1 => "1 Gerät".to_string(),
                count => format!("{count} Geräte"),
            };
            plain("👥", &room.name, Some(count))
        }
        ItemRef::Vault(vault) => {
            let (icon, status) = match vault.state.as_str() {
                "locked" => ("🔒", "Gesperrt".to_string()),
                "unlocked" => ("🔓", "Entsperrt".to_string()),
                "locking" => ("🔒", "Wird gesperrt".to_string()),
                "unavailable" => ("🔒", "Nicht verfügbar".to_string()),
                other => ("🔒", other.to_string()),
            };
            plain(icon, &vault.label, Some(status))
        }
        ItemRef::File(file) => plain(if file.is_dir { "📁" } else { "📄" }, &file.name, None),
        ItemRef::System(SystemItem::RecycleBin) => plain("🗑", SystemItem::RecycleBin.label(), None),
        ItemRef::System(SystemItem::ThisPc) => plain("🖥", SystemItem::ThisPc.label(), None),
        ItemRef::AllScreens => plain("📺", "Alle Bildschirme", None),
        ItemRef::Empty(action) => {
            let icon = match action {
                EmptyAction::AddDevice => "➕",
                EmptyAction::JoinRoom => "🚪",
            };
            plain(icon, action.label(), None)
        }
    }
}

/// How a tile is drawn this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TileState {
    pub(crate) selected: bool,
    pub(crate) hovered: bool,
    /// Cut with Ctrl+X: dimmed until pasted.
    pub(crate) dimmed: bool,
    /// The label is being renamed (the editor covers it).
    pub(crate) renaming: bool,
}

/// The rectangle of the label below the symbol (the rename editor's place).
pub(crate) fn label_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(rect.left() + 4.0, rect.top() + 58.0),
        Pos2::new(rect.right() - 4.0, rect.top() + 82.0),
    )
}

/// Paints one tile into `rect` (one grid cell).
pub(crate) fn paint(ui: &mut egui::Ui, rect: Rect, look: &TileLook, state: TileState) {
    let painter = ui.painter().clone();
    let alpha = if state.dimmed { 110 } else { 255 };
    let text_color = Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
    let inner = rect.shrink(3.0);
    if state.selected {
        painter.rect_filled(
            inner,
            6.0,
            Color32::from_rgba_unmultiplied(70, 130, 210, 110),
        );
        painter.rect_stroke(
            inner,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(120, 170, 235)),
        );
    } else if state.hovered {
        painter.rect_filled(
            inner,
            6.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 40),
        );
    }
    let icon_center = Pos2::new(rect.center().x, rect.top() + 30.0);
    painter.text(
        icon_center,
        Align2::CENTER_CENTER,
        look.icon,
        FontId::proportional(ICON_FONT),
        text_color,
    );
    let mark_center = icon_center + Vec2::new(17.0, 14.0);
    match look.mark {
        Mark::None => {}
        Mark::Dot(color) => {
            painter.circle_filled(mark_center, 6.0, color);
            painter.circle_stroke(mark_center, 6.0, Stroke::new(1.5, Color32::BLACK));
        }
        Mark::Ring(color) => {
            painter.circle_stroke(mark_center, 5.5, Stroke::new(2.5, color));
        }
        Mark::Busy => {
            let spinner = Rect::from_center_size(mark_center, Vec2::splat(14.0));
            ui.put(spinner, egui::Spinner::new().size(14.0));
        }
    }
    if !state.renaming {
        paint_label(&painter, rect, &look.title, TITLE_FONT, 2, text_color);
    }
    if let Some(status) = &look.status {
        let status_rect = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + 84.0),
            rect.right_bottom(),
        );
        let color = Color32::from_rgba_unmultiplied(220, 220, 220, alpha);
        paint_label(&painter, status_rect, status, STATUS_FONT, 1, color);
    }
}

/// Centered text of at most `rows` rows on the dark plate.
fn paint_label(
    painter: &egui::Painter,
    rect: Rect,
    text: &str,
    size: f32,
    rows: usize,
    color: Color32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_string(),
        FontId::proportional(size),
        color,
        rect.width() - 8.0,
    );
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    job.halign = egui::Align::Center;
    let galley = painter.layout_job(job);
    let top = if rows > 1 {
        label_rect(rect).top()
    } else {
        rect.top()
    };
    let origin = Pos2::new(rect.center().x, top);
    let plate = Rect::from_center_size(
        Pos2::new(origin.x, origin.y + galley.size().y / 2.0),
        galley.size() + Vec2::new(8.0, 2.0),
    );
    painter.rect_filled(plate, 3.0, PLATE);
    painter.galley(origin, galley, color);
}
