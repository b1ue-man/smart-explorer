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
use std::time::Instant;

use eframe::egui::{self, Vec2};

use super::screen_devices::{entries, DeviceEntry, Devices, GridScope};
use super::screen_pool::{
    host_pending, preview_edge, preview_fps, preview_gap_ms, preview_rate_mode, preview_request_to,
    preview_set_mode_for, DEFAULT_PREVIEW_INTERVAL_MS,
};
use super::screen_prefs::codec_offer;
use super::screen_prefs::Fit;
use super::screen_session::{Backend, ProbeInfo, ViewerSession};
use super::screen_target::ScreenTarget;
use super::screen_texture_cpu::CpuTexture;
use super::screen_tile_face::{draw_tile, Face};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::ViewerFrame;
use crate::share::access_grant::CapabilityDecision;

const MIN_WIDTH: f32 = 160.0;
const MAX_WIDTH: f32 = 480.0;
const DEFAULT_WIDTH: f32 = 240.0;
/// The choices of „Aktualisierung“.
const INTERVALS: [(u64, &str); 4] = [(500, "0,5 s"), (1000, "1 s"), (2000, "2 s"), (5000, "5 s")];

type ProbeSlot = Arc<Mutex<Option<Result<ProbeInfo, String>>>>;

pub(super) enum Probe {
    Idle,
    Running(ProbeSlot),
    Done(ProbeInfo),
    Failed(String),
}

pub(super) struct Tile {
    pub(super) entry: DeviceEntry,
    pub(super) probe: Probe,
    pub(super) session: Option<ViewerSession>,
    pub(super) texture: CpuTexture,
    pub(super) picture: Option<FrameSize>,
    pub(super) accepted_at: Option<Instant>,
    pub(super) requested_edge: u32,
    /// The session ended with an error; a click opens a new one.
    pub(super) failed: Option<String>,
    pub(super) limit: bool,
    pub(super) pan: Vec2,
    pub(super) wait_frame: bool,
    pub(super) visibility: super::screen_visibility::VisibilityTracker,
}

impl Tile {
    pub(super) fn new(entry: DeviceEntry) -> Self {
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
            pan: Vec2::ZERO,
            wait_frame: false,
            visibility: super::screen_visibility::VisibilityTracker::default(),
        }
    }
}

/// The group view of a scope.
pub struct ScreenTileGrid {
    scope: GridScope,
    backend: Backend,
    devices: Devices,
    tiles: Vec<Tile>,
    generation: u64,
    width: f32,
    interval_ms: u64,
    applied_interval: u64,
    /// A full session that returns to its tile.
    pending: Option<(ViewerSession, CpuTexture)>,
    fit: Fit,
}

impl ScreenTileGrid {
    pub fn new(scope: GridScope, backend: Backend, devices: Devices) -> Self {
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
            fit: Fit::Fit,
        }
    }

    /// The preview connection of a tile, for the full view of the same device.
    pub fn take_session(&mut self, target: &ScreenTarget) -> Option<(ViewerSession, CpuTexture)> {
        let tile = self
            .tiles
            .iter_mut()
            .find(|tile| tile.entry.target.same_device(target))?;
        if let Some(session) = &tile.session {
            session.invalidate_input();
        }
        tile.picture = None;
        tile.texture.reset();
        tile.session
            .take()
            .map(|session| (session, std::mem::take(&mut tile.texture)))
    }

    /// The full view going back to its tile as a preview; it is placed once
    /// the tiles exist.
    pub fn adopt(&mut self, session: ViewerSession, texture: CpuTexture) {
        self.pending = Some((session, texture));
        self.generation = u64::MAX;
    }

    fn place_pending(&mut self) {
        let Some((session, texture)) = self.pending.take() else {
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
            tile.visibility.invalidate();
            tile.texture = texture;
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
        tile.texture.reset();
        tile.visibility.invalidate();
        // The stored „Codec“ as this tile's host serves it (user rule
        // 2026-11-24, `screen_pool::open_choice`).
        let target = &tile.entry.target;
        let request =
            preview_request_to(Some(target), &self.backend.offers, edge, self.interval_ms);
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
    pub(super) fn step(&self, tile: &mut Tile, ctx: &egui::Context, edge: u32) {
        if !tile.entry.online {
            tile.session = None;
            tile.picture = None;
            tile.texture.reset();
            tile.visibility.invalidate();
            tile.wait_frame = false;
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
                let closed = tile.session.is_none() && tile.failed.is_none() && !tile.limit;
                // A preview whose stored choices depend on the host waits
                // for its encoders; a tile that asks learns them for the
                // full view a click opens.
                let asks = info.supported && info.view == CapabilityDecision::Ask;
                let (offers, wake) = (&self.backend.offers, self.backend.wake.clone());
                let pending =
                    ((live && closed) || asks) && host_pending(&tile.entry.target, offers, wake);
                if live && closed && !pending {
                    self.open_preview(tile, edge);
                }
            }
            Probe::Failed(_) => {}
        }
        let Some(session) = tile.session.as_ref() else {
            return;
        };
        // Under the preview's gap a picture waits for its end (Codex 12/13);
        // the paint then comes by itself, also with no further picture.
        let due = session.take_due_picture().unwrap_or_else(|wait| {
            ctx.request_repaint_after(wait);
            None
        });
        if let Some(picture) = due {
            if let Some(size) = accept_preview_picture(session, &mut tile.texture, ctx, picture) {
                tile.picture = Some(size);
                tile.wait_frame = false;
            }
        } else if tile.picture.is_some() {
            // The tile draws its picture again (r1).
            session.note_repeated();
        }
        let (ended, accepted) = {
            let info = session.info();
            (info.end.clone(), info.accepted > 0)
        };
        if accepted && tile.accepted_at.is_none() {
            tile.accepted_at = Some(Instant::now());
            if let Some(hello) = session.info().hello.as_ref() {
                codec_offer::note_hello(&tile.entry.target, hello);
            }
            // The chosen „Aktualisierung“ when it is not the host's own one
            // picture a second (Codex 11/13: it stayed at the opening's).
            if preview_fps(self.interval_ms) != 1 {
                let codec = session.shared.codec_choice();
                session.send(ViewerFrame::SetMode(preview_rate_mode(
                    self.interval_ms,
                    codec,
                )));
            }
        }
        if let Some(end) = ended {
            if end.code.as_deref() == Some("session_limit") {
                tile.limit = true;
            } else if end.code.as_deref() != Some("viewer_closed") {
                tile.failed = Some(end.text);
            }
            tile.session = None;
            tile.picture = None;
            tile.texture.reset();
            tile.visibility.invalidate();
            tile.wait_frame = false;
        } else if edge != tile.requested_edge && accepted {
            let codec = session.shared.codec_choice();
            session.send(ViewerFrame::SetMode(preview_set_mode_for(
                edge,
                self.interval_ms,
                codec,
            )));
            tile.requested_edge = edge;
        }
    }

    /// Draws the group view; the target of the tile the user clicked.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<ScreenTarget> {
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
                        if let Some(target) = draw_tile(ui, tile, width, self.fit) {
                            clicked = Some(target);
                        }
                    }
                });
            });
        self.tiles = tiles;
        // The full view opens for this device (`screen_pool::view_choice`).
        if let Some(target) = &clicked {
            codec_offer::set_view(target);
        }
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
        // The row break of the viewer's toolbar (Gemini 8/11): egui breaks
        // no row before a list, which ran past the window's edge here.
        use super::screen_toolbar_ui::wrap;
        ui.horizontal_wrapped(|ui| {
            wrap::begin(ui);
            let current = INTERVALS
                .iter()
                .find(|(ms, _)| *ms == self.interval_ms)
                .map_or("1 s", |(_, label)| *label);
            wrap::reserve_pair(ui, "Aktualisierung:", current);
            ui.label("Aktualisierung:");
            let list = egui::ComboBox::from_id_salt("screen_grid_interval");
            wrap::fitted_after_label(ui, list, current).show_ui(ui, |ui| {
                for (ms, label) in INTERVALS {
                    ui.selectable_value(&mut self.interval_ms, ms, label);
                }
            });
            for (fit, text) in [
                (Fit::Fit, "Einpassen"),
                (Fit::Actual, "1:1 · Ziehen zum Verschieben"),
            ] {
                wrap::reserve(ui, wrap::button_width(ui, text));
                ui.selectable_value(&mut self.fit, fit, text);
            }
            let gap = ui.spacing().item_spacing.x;
            let slider = ui.spacing().slider_width;
            wrap::reserve(ui, wrap::label_width(ui, "Größe:") + gap + slider);
            ui.label("Größe:");
            ui.add(egui::Slider::new(&mut self.width, MIN_WIDTH..=MAX_WIDTH).show_value(false));
        });
        if self.interval_ms != self.applied_interval {
            self.applied_interval = self.interval_ms;
            let edge = preview_edge(self.width, ui.ctx().pixels_per_point());
            for session in tiles.iter().filter_map(|tile| tile.session.as_ref()) {
                let codec = session.shared.codec_choice();
                session.send(ViewerFrame::SetMode(preview_set_mode_for(
                    edge,
                    self.interval_ms,
                    codec,
                )));
                session.set_present_gap_ms(preview_gap_ms(self.interval_ms));
            }
        }
    }
}

#[cfg(test)]
impl ScreenTileGrid {
    /// The faces of the tiles (tests).
    pub fn faces(&self) -> Vec<Face> {
        self.tiles.iter().map(Tile::face).collect()
    }
}

/// A visibility ACK admits a fresh decoded image before input is unlocked.
/// Only the subsequent paint callback may mark that image as presented.
fn accept_preview_picture(
    session: &ViewerSession,
    texture: &mut CpuTexture,
    ctx: &egui::Context,
    picture: super::screen_session::Picture,
) -> Option<FrameSize> {
    let hidden = ctx
        .data(|data| {
            data.get_temp::<super::screen_visibility::WindowVisibility>(egui::Id::new(
                "screen_window_visibility",
            ))
        })
        .is_some_and(|window| window.hidden());
    if hidden {
        return None;
    }
    let picture = session.presentable(picture)?;
    let super::screen_session::PictureData::Bgra(image) = picture.data else {
        return None;
    };
    super::screen_render_caps::bind_texture(texture, session, picture.memory.clone());
    texture.update_frame(ctx, &image, picture.presented);
    session.recycle_bgra(image);
    (!texture.failed()).then_some(picture.size)
}
#[cfg(test)]
#[path = "batches_preview_task_tests.rs"]
mod batches_preview_task_tests;
