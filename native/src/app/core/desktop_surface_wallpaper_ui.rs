//! The wallpaper behind the tiles (DSK-M4): the configured background or the
//! system's in its fit mode (Füllen, Einpassen, Strecken, Kacheln,
//! Zentrieren, Spannen), decoded in a thread and kept as one texture that
//! repeats for "Kacheln"; around and instead of an image its color shows,
//! `transparent` paints nothing where the session allows it. The system
//! wallpaper arrives with DSK-M5 and DSK-M6; until then "System" shows the
//! default color.
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, Color32, Pos2, Rect};

use super::desktop_surface_host::{self as host_ui, SurfaceHost};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::os::wallpaper_image::{self, Area, FitMode, Loaded, WallpaperSpec};

/// Longest side of a decoded wallpaper, within the texture limit.
const MAX_IMAGE_SIDE: usize = 8_192;

#[derive(Default)]
pub(crate) struct Wallpaper {
    spec: Option<WallpaperSpec>,
    texture: Option<(egui::TextureHandle, (u32, u32))>,
    pending: Option<Receiver<Loaded>>,
}

fn color([red, green, blue]: [u8; 3]) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

fn area_of(rect: Rect) -> Area {
    Area::new(rect.left(), rect.top(), rect.width(), rect.height())
}

fn rect_of(area: Area) -> Rect {
    Rect::from_min_size(Pos2::new(area.x, area.y), egui::vec2(area.w, area.h))
}

/// Every monitor together in points of this surface ("Spannen").
fn span_area(ctx: &egui::Context, screen: Rect, host: &dyn SurfaceHost) -> Option<Area> {
    let monitor = host_ui::monitor_rect(host)?;
    let all = host_ui::virtual_desktop(host)?;
    let ppp = ctx.pixels_per_point();
    Some(Area::new(
        screen.left() + (all[0] - monitor[0]) as f32 / ppp,
        screen.top() + (all[1] - monitor[1]) as f32 / ppp,
        (all[2] - all[0]) as f32 / ppp,
        (all[3] - all[1]) as f32 / ppp,
    ))
}

impl Wallpaper {
    /// Reads the image again ("Aktualisieren", F5).
    pub(crate) fn reload(&mut self) {
        self.spec = None;
    }

    /// Paints the background of `config` over `screen`; `transparent`: the
    /// session gives the surface an alpha channel.
    pub(crate) fn paint(
        &mut self,
        ui: &egui::Ui,
        config: &DesktopConfig,
        transparent: bool,
        screen: Rect,
        host: &dyn SurfaceHost,
    ) {
        let allowed: Result<(), String> = if transparent {
            Ok(())
        } else {
            Err(String::new())
        };
        let background = config.effective_background(&allowed);
        let spec = wallpaper_image::spec_for(&background, None);
        if self.spec.as_ref() != Some(&spec) {
            self.start(ui.ctx(), &spec);
            self.spec = Some(spec.clone());
        }
        self.receive(ui.ctx(), &spec);
        let painter = ui.painter();
        match &spec {
            WallpaperSpec::Transparent => {}
            WallpaperSpec::Color(rgb) => {
                painter.rect_filled(screen, 0.0, color(*rgb));
            }
            WallpaperSpec::Image {
                mode, color: rgb, ..
            } => {
                painter.rect_filled(screen, 0.0, color(*rgb));
                let Some((texture, size)) = &self.texture else {
                    return;
                };
                let span = span_area(ui.ctx(), screen, host);
                let ppp = ui.ctx().pixels_per_point();
                if let Some(placement) =
                    wallpaper_image::place(*mode, *size, area_of(screen), ppp, span)
                {
                    let uv = rect_of(placement.uv);
                    painter.image(texture.id(), rect_of(placement.dest), uv, Color32::WHITE);
                }
            }
        }
    }

    /// Decodes the image of `spec` in a thread; the color shows meanwhile.
    fn start(&mut self, ctx: &egui::Context, spec: &WallpaperSpec) {
        self.texture = None;
        self.pending = None;
        let WallpaperSpec::Image { path, .. } = spec else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let path = path.clone();
        let max_side = ctx
            .input(|input| input.max_texture_side)
            .min(MAX_IMAGE_SIDE) as u32;
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("se-desktop-wallpaper".into())
            .spawn(move || {
                let _ = sender.send(wallpaper_image::load(Some(&path), max_side));
                wake.request_repaint();
            });
        if spawned.is_ok() {
            self.pending = Some(receiver);
        }
    }

    fn receive(&mut self, ctx: &egui::Context, spec: &WallpaperSpec) {
        let Some(pending) = &self.pending else {
            return;
        };
        let loaded = match pending.try_recv() {
            Ok(loaded) => loaded,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                return;
            }
        };
        self.pending = None;
        // A background without decodable image keeps its color (Annahme A9).
        let (WallpaperSpec::Image { mode, .. }, Loaded::Image(image)) = (spec, loaded) else {
            return;
        };
        let size = [image.width as usize, image.height as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
        let options = if *mode == FitMode::Tile {
            egui::TextureOptions::LINEAR_REPEAT
        } else {
            egui::TextureOptions::LINEAR
        };
        let texture = ctx.load_texture("se-desktop-wallpaper", pixels, options);
        self.texture = Some((texture, (image.width, image.height)));
    }
}
