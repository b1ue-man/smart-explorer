//! `SurfaceHost` of one layer output (DSK-M7, U37-13). The view is the one
//! of the eframe windows; this host tells it where the surface lies. The
//! monitors are the outputs of the session in the compositor's logical
//! coordinates (the layer session only). The surface is the work area of its
//! output (anchored to all edges, exclusive zone 0), so `work_area` names
//! none and the view uses the whole surface. A pointer held down keeps
//! reporting to the surface that grabbed it, also beyond its edge, so a tile
//! dragged to another output is found by [`LayerHost::global_point`].
use std::sync::Arc;

use eframe::egui;

use crate::app::desktop_surface_clipboard::FileClipboard;
use crate::app::desktop_surface_host::{NativeWindow, SurfaceHost, SystemBackground};
use crate::desktop::os::{platform, Monitor};

pub(super) struct LayerHost {
    key: String,
    monitors: Vec<Monitor>,
    /// Logical position of the output in the compositor's space.
    origin: [i32; 2],
    ctx: egui::Context,
    clipboard: Option<Arc<dyn FileClipboard>>,
}

impl LayerHost {
    pub(super) fn new(
        key: &str,
        monitors: Vec<Monitor>,
        origin: [i32; 2],
        ctx: egui::Context,
        clipboard: Option<Arc<dyn FileClipboard>>,
    ) -> LayerHost {
        LayerHost {
            key: key.to_string(),
            monitors,
            origin,
            ctx,
            clipboard,
        }
    }

    /// The point of the compositor's space under a position of this
    /// surface, rounded to whole logical pixels.
    pub(super) fn point_of(origin: [i32; 2], pointer: [f32; 2]) -> [i32; 2] {
        [
            origin[0].saturating_add(pointer[0].round() as i32),
            origin[1].saturating_add(pointer[1].round() as i32),
        ]
    }
}

impl SurfaceHost for LayerHost {
    fn monitor_key(&self) -> &str {
        &self.key
    }

    fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    fn work_area(&self) -> Option<[i32; 4]> {
        None
    }

    fn native_window(&self) -> Option<NativeWindow> {
        Some(NativeWindow::Wayland)
    }

    fn request_repaint(&self) {
        self.ctx.request_repaint();
    }

    fn monitor_name(&self, key: &str) -> Option<String> {
        Some(key.to_string())
    }

    fn system_background(&self) -> Result<SystemBackground, String> {
        platform::system_wallpaper(&self.key)
    }

    fn global_point(&self, pointer: [f32; 2]) -> Option<[i32; 2]> {
        Some(LayerHost::point_of(self.origin, pointer))
    }

    fn file_clipboard(&self) -> Option<Arc<dyn FileClipboard>> {
        self.clipboard.clone()
    }
}
