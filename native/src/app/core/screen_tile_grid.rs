//! The group view of the desktop viewer (SCR-M7, U25-10, decision C6): one
//! tile per device, with a preview of its screen. Per device one probe asks
//! what the device allows; „Bildschirm ansehen = Erlauben“ starts a preview
//! session (the host sets the pace: one picture per interval), „Fragen“
//! shows „Ansehen anfragen“ and no preview, and a device that is not online
//! is not connected to. A click on a tile returns its target; the viewer app
//! then raises the tile's own connection to the full view (`screen_pool`).
//! There is no fixed limit of tiles: a device that runs its most sessions
//! says `session_limit`, and its tile stays closed until it is clicked.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Vec2};

use super::screen_devices::{entries, DeviceEntry, Devices, GridScope};
use super::screen_pool::{
    preview_edge, preview_gap_ms, preview_request, preview_set_mode, DEFAULT_PREVIEW_INTERVAL_MS,
};
use super::screen_session::{Backend, ProbeInfo, ViewerSession};
use super::screen_target::ScreenTarget;
use super::screen_texts::state_text;
use super::screen_texture_cpu::CpuTexture;
use super::screen_tile_face::Face;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::ViewerFrame;
use crate::share::access_grant::CapabilityDecision;

const MIN_WIDTH: f32 = 160.0;
const MAX_WIDTH: f32 = 480.0;
const DEFAULT_WIDTH: f32 = 240.0;
/// A tile that got no picture this long after the host accepted it says so
/// (the preview interval is at most 5 s, plus transfer: twice that).
const NO_ANSWER: Duration = Duration::from_secs(10);
/// The choices of „Aktualisierung“.
const INTERVALS: [(u64, &str); 4] = [(500, "0,5 s"), (1000, "1 s"), (2000, "2 s"), (5000, "5 s")];
const LABEL_HEIGHT: f32 = 24.0;

type ProbeSlot = Arc<Mutex<Option<Result<ProbeInfo, String>>>>;

enum Probe {
    Idle,
    Running(ProbeSlot),
    Done(ProbeInfo),
    Failed(String),
}

struct Tile {
    entry: DeviceEntry,
    probe: Probe,
    session: Option<ViewerSession>,
    texture: CpuTexture,
    picture: Option<FrameSize>,
    accepted_at: Option<Instant>,
    requested_edge: u32,
    /// The session ended with an error; a click opens a new one.
    failed: Option<String>,
    limit: bool,
}

impl Tile {
    fn new(entry: DeviceEntry) -> Self {
        Self {
            entry,
            probe: Probe::Idle,
            session: None,
            texture: CpuTexture::default(),
            picture: None,
            accepted_at: None,
            requested_edge: 0,
            failed: None,
            limit: false,
        }
    }

    fn face(&self) -> Face {
        if !self.entry.online {
            return Face::Offline;
        }
        match &self.probe {
            Probe::Idle | Probe::Running(_) => return Face::Probing,
            Probe::Failed(error) => return Face::ProbeFailed(error.clone()),
            Probe::Done(info) if !info.supported => return Face::Unsupported,
            Probe::Done(info) if info.view == CapabilityDecision::Off => return Face::NotShared,
            Probe::Done(info) if info.view == CapabilityDecision::Ask => return Face::Ask,
            Probe::Done(_) => {}
        }
        if self.limit {
            return Face::Limit;
        }
        if let Some(error) = &self.failed {
            return Face::Failed(error.clone());
        }
        if self.picture.is_some() {
            return Face::Picture;
        }
        let Some(session) = &self.session else {
            return Face::Waiting(state_text(SessionState::Connecting, None));
        };
        let info = session.info();
        if self
            .accepted_at
            .is_some_and(|accepted| accepted.elapsed() >= NO_ANSWER)
        {
            return Face::NoAnswer;
        }
        let text = state_text(info.state, info.reason.as_deref());
        Face::Waiting(if text.is_empty() {
            "Warte auf das erste Bild …".to_string()
        } else {
            text
        })
    }
}

/// The group view of a scope.
pub(crate) struct ScreenTileGrid {
    scope: GridScope,
    backend: Backend,
    devices: Devices,
    tiles: Vec<Tile>,
    generation: u64,
    width: f32,
    interval_ms: u64,
    applied_interval: u64,
    /// A full session that returns to its tile.
    pending: Option<ViewerSession>,
}

impl ScreenTileGrid {
    pub(crate) fn new(scope: GridScope, backend: Backend, devices: Devices) -> Self {
        Self {
            scope,
            backend,
            devices,
            tiles: Vec::new(),
            generation: 0,
            width: DEFAULT_WIDTH,
            interval_ms: DEFAULT_PREVIEW_INTERVAL_MS,
            applied_interval: DEFAULT_PREVIEW_INTERVAL_MS,
            pending: None,
        }
    }

    /// The preview connection of a tile, for the full view of the same device.
    pub(crate) fn take_session(&mut self, target: &ScreenTarget) -> Option<ViewerSession> {
        let tile = self
            .tiles
            .iter_mut()
            .find(|tile| tile.entry.target.same_device(target))?;
        tile.picture = None;
        tile.session.take()
    }

    /// The full view going back to its tile as a preview; it is placed once
    /// the tiles exist.
    pub(crate) fn adopt(&mut self, session: ViewerSession) {
        self.pending = Some(session);
        self.generation = u64::MAX;
    }

    fn place_pending(&mut self) {
        let Some(session) = self.pending.take() else {
            return;
        };
        let info = session.info().hello.as_ref().map(|hello| ProbeInfo {
            supported: true,
            view: hello.authorization.view,
        });
        if let Some(tile) = self
            .tiles
            .iter_mut()
            .find(|tile| tile.entry.target.same_device(&session.target))
        {
            if let Some(info) = info {
                tile.probe = Probe::Done(info);
            }
            tile.accepted_at = Some(Instant::now());
            tile.session = Some(session);
        }
    }

    /// Follows the devices of the profiles: new ones get a tile, gone ones
    /// lose theirs with their session.
    fn sync_devices(&mut self) {
        let state = self.devices.lock();
        if state.generation == self.generation {
            return;
        }
        self.generation = state.generation;
        let Some(profiles) = state.profiles.as_ref() else {
            return;
        };
        let now = crate::share::core_now_secs();
        let wanted = self.scope.select(profiles, entries(profiles, now));
        let mut old = std::mem::take(&mut self.tiles);
        self.tiles = wanted
            .into_iter()
            .map(|entry| {
                match old
                    .iter()
                    .position(|tile| tile.entry.target.same_device(&entry.target))
                {
                    Some(index) => {
                        let mut tile = old.swap_remove(index);
                        tile.entry = entry;
                        tile
                    }
                    None => Tile::new(entry),
                }
            })
            .collect();
        drop(state);
        self.place_pending();
    }

    fn start_probe(&self, tile: &mut Tile) {
        let Ok(peer) = tile.entry.target.to_peer() else {
            tile.probe = Probe::Failed("Ziel ungültig".into());
            return;
        };
        let slot: ProbeSlot = Arc::new(Mutex::new(None));
        let (probe, wake, result) = (
            self.backend.probe.clone(),
            self.backend.wake.clone(),
            slot.clone(),
        );
        let started = std::thread::Builder::new()
            .name("screen-viewer-probe".into())
            .spawn(move || {
                let answer = probe(peer);
                *result.lock().unwrap_or_else(PoisonError::into_inner) = Some(answer);
                wake();
            });
        tile.probe = match started {
            Ok(_) => Probe::Running(slot),
            Err(error) => Probe::Failed(error.to_string()),
        };
    }

    fn open_preview(&self, tile: &mut Tile, edge: u32) {
        let request = preview_request(&self.backend.offers, edge, self.interval_ms);
        let session = ViewerSession::open(
            &self.backend,
            tile.entry.target.clone(),
            request,
            false,
            false,
            preview_gap_ms(self.interval_ms),
        );
        tile.requested_edge = edge;
        tile.accepted_at = None;
        tile.session = Some(session);
    }

    /// One step of a tile: probe, session, picture.
    fn step(&self, tile: &mut Tile, ctx: &egui::Context, edge: u32) {
        if !tile.entry.online {
            tile.session = None;
            tile.picture = None;
            tile.failed = None;
            if matches!(tile.probe, Probe::Done(_) | Probe::Failed(_)) {
                tile.probe = Probe::Idle;
            }
            return;
        }
        match &tile.probe {
            Probe::Idle => self.start_probe(tile),
            Probe::Running(slot) => {
                let answer = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
                match answer {
                    Some(Ok(info)) => tile.probe = Probe::Done(info),
                    Some(Err(error)) => tile.probe = Probe::Failed(error),
                    None => {}
                }
            }
            Probe::Done(info) => {
                let live = info.supported && info.view == CapabilityDecision::Allow;
                if live && tile.session.is_none() && tile.failed.is_none() && !tile.limit {
                    self.open_preview(tile, edge);
                }
            }
            Probe::Failed(_) => {}
        }
        let Some(session) = tile.session.as_ref() else {
            return;
        };
        if let Some(picture) = session.take_picture() {
            if let super::screen_session::PictureData::Bgra(image) = picture.data {
                tile.texture.update(ctx, &image);
                tile.picture = Some(picture.size);
            }
        }
        let (ended, accepted) = {
            let info = session.info();
            (info.end.clone(), info.accepted > 0)
        };
        if accepted && tile.accepted_at.is_none() {
            tile.accepted_at = Some(Instant::now());
        }
        if let Some(end) = ended {
            if end.code.as_deref() == Some("session_limit") {
                tile.limit = true;
            } else if end.code.as_deref() != Some("viewer_closed") {
                tile.failed = Some(end.text);
            }
            tile.session = None;
        } else if edge != tile.requested_edge && accepted {
            session.send(ViewerFrame::SetMode(preview_set_mode(
                edge,
                self.interval_ms,
            )));
            tile.requested_edge = edge;
        }
    }

    /// Draws the group view; the target of the tile the user clicked.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> Option<ScreenTarget> {
        self.sync_devices();
        let ctx = ui.ctx().clone();
        let edge = preview_edge(self.width, ctx.pixels_per_point());
        let mut tiles = std::mem::take(&mut self.tiles);
        for tile in &mut tiles {
            self.step(tile, &ctx, edge);
        }
        self.controls(ui, &mut tiles);
        let width = self.width;
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut group: Option<String> = None;
                ui.horizontal_wrapped(|ui| {
                    for tile in &mut tiles {
                        let room = tile.entry.room.clone();
                        if room != group {
                            if let Some(name) = &room {
                                ui.end_row();
                                ui.heading(name);
                                ui.end_row();
                            }
                            group = room;
                        }
                        if let Some(target) = draw_tile(ui, tile, width) {
                            clicked = Some(target);
                        }
                    }
                });
            });
        self.tiles = tiles;
        let message = {
            let state = self.devices.lock();
            match state.profiles.as_ref() {
                None => Some("Lese die Geräte …"),
                Some(profiles) if !self.scope.exists(profiles) => {
                    Some("Dieser Raum ist nicht gespeichert.")
                }
                Some(_) if self.tiles.is_empty() => Some("Keine Geräte mit Bildschirmfreigabe."),
                Some(_) => None,
            }
        };
        if let Some(message) = message {
            ui.label(message);
        }
        clicked
    }

    /// „Aktualisierung“ and „Größe“.
    fn controls(&mut self, ui: &mut egui::Ui, tiles: &mut [Tile]) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Aktualisierung:");
            let current = INTERVALS
                .iter()
                .find(|(ms, _)| *ms == self.interval_ms)
                .map_or("1 s", |(_, label)| *label);
            egui::ComboBox::from_id_salt("screen_grid_interval")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (ms, label) in INTERVALS {
                        ui.selectable_value(&mut self.interval_ms, ms, label);
                    }
                });
            ui.label("Größe:");
            ui.add(egui::Slider::new(&mut self.width, MIN_WIDTH..=MAX_WIDTH).show_value(false));
        });
        if self.interval_ms != self.applied_interval {
            self.applied_interval = self.interval_ms;
            let edge = preview_edge(self.width, ui.ctx().pixels_per_point());
            for session in tiles.iter().filter_map(|tile| tile.session.as_ref()) {
                session.send(ViewerFrame::SetMode(preview_set_mode(
                    edge,
                    self.interval_ms,
                )));
                session.set_present_gap_ms(preview_gap_ms(self.interval_ms));
            }
        }
    }
}

/// Draws one tile; its target when the user clicked a tile that can be opened.
fn draw_tile(ui: &mut egui::Ui, tile: &mut Tile, width: f32) -> Option<ScreenTarget> {
    let face = tile.face();
    let image_height = width * 9.0 / 16.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, image_height + LABEL_HEIGHT),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);
    let image_area = Rect::from_min_size(rect.min, Vec2::new(width, image_height));
    painter.rect_filled(image_area, 4.0, Color32::from_gray(24));
    match (&face, tile.picture, tile.texture.id()) {
        (Face::Picture, Some(size), Some(texture)) => {
            let scale = (width / size.width as f32).min(image_height / size.height as f32);
            let dest = Rect::from_center_size(
                image_area.center(),
                Vec2::new(size.width as f32 * scale, size.height as f32 * scale),
            );
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture, dest, uv, Color32::WHITE);
            let streaming = tile
                .session
                .as_ref()
                .is_none_or(|session| session.info().state == SessionState::Streaming);
            if !streaming {
                painter.rect_filled(dest, 0.0, Color32::from_black_alpha(128));
            }
        }
        _ => {
            let galley = painter.layout(
                face.text(),
                FontId::proportional(13.0),
                Color32::from_gray(200),
                width - 16.0,
            );
            let at = image_area.center() - galley.size() / 2.0;
            painter.galley(at, galley, Color32::from_gray(200));
        }
    }
    let label_center = Pos2::new(rect.min.x + 6.0, rect.max.y - LABEL_HEIGHT / 2.0);
    painter.circle_filled(
        Pos2::new(rect.max.x - 10.0, label_center.y),
        5.0,
        face.dot(),
    );
    painter.text(
        label_center,
        Align2::LEFT_CENTER,
        &tile.entry.name,
        FontId::proportional(13.0),
        ui.visuals().text_color(),
    );
    let openable = matches!(face, Face::Picture | Face::Ask);
    let retry = matches!(face, Face::ProbeFailed(_) | Face::Failed(_) | Face::Limit);
    if openable || retry {
        response.clone().on_hover_cursor(CursorIcon::PointingHand);
    }
    if !response.clicked() {
        return None;
    }
    if retry {
        tile.probe = Probe::Idle;
        tile.failed = None;
        tile.limit = false;
        tile.session = None;
        tile.picture = None;
        return None;
    }
    openable.then(|| tile.entry.target.clone())
}

#[cfg(test)]
impl ScreenTileGrid {
    /// The faces of the tiles (tests).
    pub(crate) fn faces(&self) -> Vec<Face> {
        self.tiles.iter().map(Tile::face).collect()
    }
}
