//! What a tile of the group view shows (SCR-M7, U25-10): the state of the
//! device and of its preview, the text on the tile and the color of the dot
//! beside the name (green: a picture, yellow: waiting, red: an error, gray:
//! nothing to show).
use super::screen_prefs::Fit;
use super::screen_target::ScreenTarget;
use super::screen_tile_grid::{Probe, Tile};
use crate::screen::core::session_state::SessionState;
use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Vec2};
const LABEL_HEIGHT: f32 = 24.0;

use super::screen_texts::{error_text, state_text};
use crate::share::access_grant::CapabilityDecision;
use std::time::Duration;
/// A tile that got no picture this long after the host accepted it says so
/// (the preview interval is at most 5 s, plus transfer: twice that).
const NO_ANSWER: Duration = Duration::from_secs(10);

/// What a tile shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    Offline,
    Probing,
    ProbeFailed(String),
    Unsupported,
    NotShared,
    Ask,
    Waiting(String),
    Picture,
    NoAnswer,
    Failed(String),
    Limit,
}

impl Face {
    pub(crate) fn dot(&self) -> Color32 {
        match self {
            Self::Picture => Color32::from_rgb(70, 190, 100),
            Self::Probing | Self::Ask | Self::Waiting(_) | Self::NoAnswer => {
                Color32::from_rgb(230, 190, 60)
            }
            Self::ProbeFailed(_) | Self::Failed(_) | Self::Limit => Color32::from_rgb(220, 80, 70),
            Self::Offline | Self::Unsupported | Self::NotShared => Color32::from_gray(120),
        }
    }

    pub(crate) fn text(&self) -> String {
        match self {
            Self::Offline => "Offline".into(),
            Self::Probing => "Prüfe …".into(),
            Self::ProbeFailed(error) => format!("Nicht erreichbar: {error}"),
            Self::Unsupported => "Keine Bildschirmfreigabe (ältere Version)".into(),
            Self::NotShared => "Bildschirm nicht freigegeben".into(),
            Self::Ask => "Ansehen anfragen".into(),
            Self::Waiting(text) | Self::Failed(text) => text.clone(),
            Self::Picture => String::new(),
            Self::NoAnswer => "Keine Antwort".into(),
            Self::Limit => error_text("session_limit", None),
        }
    }
}

/// Draws one tile; its target when the user clicked a tile that can be opened.
pub(crate) fn draw_tile(
    ui: &mut egui::Ui,
    tile: &mut Tile,
    width: f32,
    fit: Fit,
) -> Option<ScreenTarget> {
    let mut window = ui
        .ctx()
        .data(|data| {
            data.get_temp::<super::screen_visibility::WindowVisibility>(egui::Id::new(
                "screen_window_visibility",
            ))
        })
        .unwrap_or_default();
    if window.hidden() {
        tile.picture = None;
        tile.texture.reset();
        tile.wait_frame = true;
    }
    let face = tile.face();
    let image_height = width * 9.0 / 16.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, image_height + LABEL_HEIGHT),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    let image_area = Rect::from_min_size(rect.min, Vec2::new(width, image_height));
    if !ui.is_rect_visible(image_area) || !image_area.intersect(ui.clip_rect()).is_positive() {
        window.shown = Some(false);
        tile.picture = None;
        tile.texture.reset();
        tile.wait_frame = true;
    }
    painter.rect_filled(image_area, 4.0, Color32::from_gray(24));
    if fit == Fit::Actual && response.dragged() && tile.picture.is_some() {
        tile.pan -= ui.input(|input| input.pointer.delta());
    }
    if let Some(session) = &tile.session {
        session.refresh_visibility();
        let geometry = tile
            .picture
            .or_else(|| session.info().stream.as_ref().map(|stream| stream.size))
            .map(|size| super::screen_input_map::Geometry {
                rect: super::screen_view_widget::image_rect(
                    image_area,
                    size,
                    fit,
                    ui.ctx().pixels_per_point(),
                    tile.pan,
                ),
                size,
            });
        if let Some(report) = tile.visibility.report(
            &window,
            image_area.intersect(ui.clip_rect()),
            geometry.as_ref(),
            ui.ctx().pixels_per_point(),
        ) {
            if report.state == crate::screen::core::visibility::VisibilityState::Hidden {
                tile.picture = None;
                tile.texture.reset();
                tile.wait_frame = true;
            }
            session.set_visibility(report);
        }
    }
    if let Some(size) = tile.picture {
        let dest = super::screen_view_widget::image_rect(
            image_area,
            size,
            fit,
            ui.ctx().pixels_per_point(),
            tile.pan,
        );
        let ppp = ui.ctx().pixels_per_point();
        tile.texture.prepare(
            ui.ctx(),
            if fit == Fit::Actual {
                [size.width as usize, size.height as usize]
            } else {
                [
                    (dest.width() * ppp).round().max(1.0) as usize,
                    (dest.height() * ppp).round().max(1.0) as usize,
                ]
            },
        );
    }
    match (&face, tile.picture) {
        (Face::Picture, Some(size)) => {
            let dest = super::screen_view_widget::image_rect(
                image_area,
                size,
                fit,
                ui.ctx().pixels_per_point(),
                tile.pan,
            );
            let painter = ui.painter_at(image_area.intersect(ui.clip_rect()));
            if let Some(session) = &tile.session {
                let drawn = !window.hidden()
                    && ui.is_rect_visible(dest)
                    && dest.intersect(painter.clip_rect()).is_positive();
                tile.texture.paint(&painter, dest, |frame| {
                    preview_receipt(session, frame, drawn)
                });
            }
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
    if let Some(notice) = tile
        .session
        .as_ref()
        .and_then(|session| session.info().notice.clone())
    {
        response.clone().on_hover_text(&notice);
        if notice.starts_with("Videoencoder nicht verfügbar;") && matches!(face, Face::Picture) {
            painter.text(
                image_area.left_top() + Vec2::splat(6.0),
                Align2::LEFT_TOP,
                "Verlustfreie Übertragung",
                FontId::proportional(12.0),
                Color32::from_rgb(255, 211, 98),
            );
        }
    }
    let openable = matches!(face, Face::Picture | Face::Ask);
    let retry = matches!(face, Face::ProbeFailed(_) | Face::Failed(_) | Face::Limit);
    if openable || retry {
        response.clone().on_hover_cursor(CursorIcon::PointingHand);
    }
    if !response.clicked() {
        return None;
    }
    if retry {
        retry(tile);
        return None;
    }
    openable.then(|| tile.entry.target.clone())
}

/// Called only after the uploaded texture was added to a visible paint pass.
pub(super) fn preview_presented(
    session: &super::screen_session::ViewerSession,
    texture: &super::screen_texture_cpu::CpuTexture,
    drawn: bool,
) -> Option<super::screen_presented::Presented> {
    if !drawn || texture.failed() {
        return None;
    }
    texture
        .presented_frame()
        .and_then(|frame| preview_receipt(session, frame, drawn))
}

fn preview_receipt(
    session: &super::screen_session::ViewerSession,
    frame: super::screen_presented::PresentedFrame,
    drawn: bool,
) -> Option<super::screen_presented::Presented> {
    (drawn && session.can_present(frame.clone())).then(|| session.presented_callback(frame))
}

/// The click path and controlled fixture use exactly the same retry retirement.
pub(super) fn retry(tile: &mut Tile) {
    tile.probe = Probe::Idle;
    tile.failed = None;
    tile.limit = false;
    tile.session = None;
    tile.picture = None;
    tile.texture.reset();
    tile.visibility.invalidate();
    tile.wait_frame = false;
}

impl Tile {
    pub(super) fn face(&self) -> Face {
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
        if self.wait_frame {
            return Face::Waiting("Warte auf ein vollständiges aktuelles Bild …".into());
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
