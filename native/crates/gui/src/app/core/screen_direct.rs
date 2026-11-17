//! The Direct3D 11 display of the desktop viewer (plan 2026-11-17
//! „Windows-Bild“ P8, like Moonlight and Shadow): a picture of FFmpeg's
//! D3D11VA stage goes from the decoder's texture through the video
//! processor into a flip-model swap chain of a child window over the
//! picture area, presented on the decode thread the moment it is decoded,
//! with no read-back, no upload and no wait for egui's next frame. The
//! wgpu/glow path stays: for every picture that comes as planes or BGRA
//! (Media Foundation, DXVA2, software, tiles), on Linux, and when the user
//! chooses „Anzeige: wgpu“ (`screen_prefs::DisplayChoice`); a presenter
//! that fails hands the session back to it.
//!
//! Receipts: a picture is presented only while `Shared::can_present`
//! allows it, and its `PRESENTED` receipt goes out after the swap chain
//! took it; a picture that may not be shown yet (the host's visibility
//! acknowledgement is outstanding, or the window has no swap chain yet) is
//! held by the presenter and shown from the window's thread later, as
//! `ViewerSession::keep_if_awaiting_floor` does for the slot.
//!
//! Overlays: the child covers egui's picture area. Menus and open popups
//! hide it while they are open; the statistics, the full-screen button,
//! tooltips and hints are cut out of its region (`screen_direct_layout`).
//! The host's pointer cannot be drawn by egui over the child: while the
//! view only watches, the host draws it into the picture again
//! (`CURSOR_OVERLAY` off); while it steers, the local system pointer stays
//! visible over the picture (the host follows it anyway).
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use eframe::egui::{self, Order, Rect};

use super::screen_direct_layout as layout;
use super::screen_presented::PresentedFrame;
use super::screen_session::{Shared, ViewerSession};
use crate::screen::core::gpu_picture::GpuPicture;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::wire_control::ViewerFrame;

#[cfg(windows)]
#[path = "../os/windows/screen_d3d11_present.rs"]
mod platform;
#[cfg(not(windows))]
mod platform {
    /// No Direct3D 11 outside Windows.
    pub struct Surface;
    impl Surface {
        pub fn new() -> std::io::Result<Self> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Direct3D 11 exists on Windows only",
            ))
        }
        pub fn presenter(&self) -> std::sync::Arc<dyn super::DirectPresenter> {
            unreachable!("no surface is ever made here")
        }
        pub fn update(
            &mut self,
            _parent: usize,
            _placement: Option<(&super::layout::DirectGeometry, bool)>,
        ) -> std::io::Result<()> {
            Ok(())
        }
    }
}

/// What draws GPU pictures straight to the screen. `owner` names the
/// session a picture belongs to (the address of its `Shared`), so a held
/// picture of an ended session is never drawn for the next one.
pub trait DirectPresenter: Send + Sync {
    /// Draws `picture` now; `false` when the window has no swap chain yet
    /// (the picture is held and drawn by `show_held`).
    fn present(&self, picture: GpuPicture, owner: usize) -> io::Result<bool>;
    /// Keeps `picture` without drawing it (replaces a held one).
    fn hold(&self, picture: GpuPicture, owner: usize);
    /// Draws the picture `owner` holds; `false` when it holds none or the
    /// window cannot draw yet.
    fn show_held(&self, owner: usize) -> io::Result<bool>;
    /// The size of the picture drawn last.
    fn picture_size(&self) -> Option<FrameSize>;
    /// The last picture of the session came this way (and none in the slot since).
    fn active(&self) -> bool;
    fn set_active(&self, on: bool);
    fn failed(&self) -> bool;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The session's link to its window's presenter (`Shared::direct`).
#[derive(Default)]
pub struct DirectSlot {
    presenter: Mutex<Option<Arc<dyn DirectPresenter>>>,
    held: Mutex<Option<PresentedFrame>>,
    /// The host draws its pointer into the picture (this view shows the
    /// picture through Direct3D 11 and does not steer).
    pointer_in_picture: AtomicBool,
}

/// The session's key for its held pictures.
fn owner(shared: &Shared) -> usize {
    shared as *const Shared as usize
}

impl DirectSlot {
    /// The presenter while it works.
    pub fn get(&self) -> Option<Arc<dyn DirectPresenter>> {
        lock(&self.presenter)
            .clone()
            .filter(|presenter| !presenter.failed())
    }

    fn set(&self, presenter: Option<Arc<dyn DirectPresenter>>) {
        let attached = lock(&self.presenter).as_ref().map(Arc::as_ptr);
        let next = presenter.as_ref().map(Arc::as_ptr);
        if attached.map(|p| p as *const ()) != next.map(|p| p as *const ()) {
            *lock(&self.presenter) = presenter;
            *lock(&self.held) = None;
        }
    }

    /// Whether the decoder should keep its pictures on the GPU.
    pub fn wanted(&self) -> bool {
        self.get().is_some()
    }

    /// `CURSOR_OVERLAY` as the session asks for it now (P6; the events of
    /// a new connection send it, `screen_decode_events.rs`).
    pub fn cursor_overlay(&self) -> bool {
        !self.pointer_in_picture.load(Ordering::Acquire)
    }
}

/// The `PRESENTED` receipt of `frame` (as `ViewerSession::presented_callback`).
fn receipt(
    shared: &Shared,
    send: &(dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync),
    frame: PresentedFrame,
) {
    let modern = shared.modern();
    let (gop_id, frame_id) = (frame.gop_id, frame.frame_id);
    shared.presented(frame, || {
        !modern || send(&ViewerFrame::Presented { gop_id, frame_id }).is_ok()
    });
}

/// The decode thread hands a GPU picture over; `false` when no presenter
/// took it (the session falls back to planes from the next picture on).
pub(super) fn deliver(
    shared: &Shared,
    send: &(dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync),
    picture: GpuPicture,
    presented: PresentedFrame,
) -> bool {
    let Some(presenter) = shared.direct.get() else {
        return false;
    };
    shared.update(|info| {
        info.latency.on_put(false);
        info.latency.on_taken();
    });
    if !shared.can_present(&presented) {
        presenter.hold(picture, owner(shared));
        *lock(&shared.direct.held) = Some(presented);
        return true;
    }
    match presenter.present(picture, owner(shared)) {
        Ok(true) => receipt(shared, send, presented),
        Ok(false) => *lock(&shared.direct.held) = Some(presented),
        Err(error) => {
            shared.direct.set(None);
            let text = format!("Direct3D 11: {error}; Anzeige über wgpu");
            shared.update(|info| info.notice = Some(text));
            return false;
        }
    }
    true
}

/// Whether the window of `ctx` shows a picture through Direct3D 11 now
/// (the statistics line, `screen_present::window_line`).
pub fn showing(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(egui::Id::new("screen_direct_showing")))
        .unwrap_or(false)
}

/// Where the picture area of the full view is this frame.
#[derive(Clone, Copy, Debug)]
pub struct DirectPlace {
    pub area: Rect,
    /// The picture in the area (`screen_viewport::image_rect`), when known.
    pub dest: Option<Rect>,
    /// The area is on screen and the session streams.
    pub visible: bool,
    /// This view's input goes to the host (the local pointer is the one).
    pub steering: bool,
}

/// The full view's side of the Direct3D 11 display, on the window's thread.
#[derive(Default)]
pub struct DirectView {
    surface: Option<platform::Surface>,
    /// Making the presenter failed: this window stays on wgpu/glow.
    unavailable: bool,
    /// The `CURSOR_OVERLAY` wish sent last.
    overlay: Option<bool>,
}

impl DirectView {
    fn presenter(&self) -> Option<Arc<dyn DirectPresenter>> {
        self.surface.as_ref().map(platform::Surface::presenter)
    }

    /// The presenter shows the session's picture.
    pub fn showing(&self) -> bool {
        self.presenter().is_some_and(|p| p.active() && !p.failed())
    }

    /// The size of the picture it shows or holds (the widget's own
    /// picture, when it has one, comes first).
    pub fn size(&self) -> Option<FrameSize> {
        self.presenter().and_then(|p| p.picture_size())
    }

    /// A picture came through the slot (planes, BGRA), or the view retired:
    /// egui draws again, the child hides.
    pub fn deactivate(&self) {
        if let Some(presenter) = self.presenter() {
            presenter.set_active(false);
        }
    }

    fn detach(&mut self, session: &ViewerSession) {
        session.shared.direct.set(None);
        self.surface = None;
    }

    /// One frame of the view: attaches the presenter to `session` while
    /// the display choice allows it, shows a held picture when it may be
    /// shown, and places the child over `place.area` with the picture at
    /// `place.dest` (points); `overlays` are rectangles egui draws over it.
    pub fn frame(
        &mut self,
        ui: &egui::Ui,
        session: &ViewerSession,
        place: DirectPlace,
        overlays: &[Rect],
    ) {
        let DirectPlace {
            area,
            dest,
            visible,
            steering,
        } = place;
        let ctx = ui.ctx().clone();
        let parent = ctx
            .data(|data| data.get_temp::<usize>(egui::Id::new("screen_parent_hwnd")))
            .unwrap_or(0);
        let enabled = super::screen_prefs::display_choice().direct() && !self.unavailable;
        if !enabled || parent == 0 {
            self.detach(session);
        } else if self.surface.is_none() {
            match platform::Surface::new() {
                Ok(surface) => self.surface = Some(surface),
                Err(error) => {
                    self.unavailable = true;
                    eprintln!("Bildschirm: Direct3D 11 nicht verfügbar: {error}");
                }
            }
        }
        if let Some(presenter) = self.presenter() {
            if presenter.failed() {
                self.unavailable = true;
                self.detach(session);
            } else {
                session.shared.direct.set(Some(presenter.clone()));
                self.show_held(session, presenter.as_ref());
            }
        }
        let placement = self.placement(&ctx, area, dest, overlays, visible);
        if let Some(surface) = self.surface.as_mut() {
            let placement = placement.as_ref().map(|(geometry, show)| (geometry, *show));
            if let Err(error) = surface.update(parent, placement) {
                eprintln!("Bildschirm: Direct3D-11-Fenster: {error}");
                self.unavailable = true;
                self.detach(session);
            }
        }
        let showing = self.showing();
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("screen_direct_showing"), showing));
        // The pointer: drawn by the host while this view only watches.
        let in_picture = showing && !steering;
        let direct = &session.shared.direct;
        direct
            .pointer_in_picture
            .store(in_picture, Ordering::Release);
        if session.shared.wants_cursor() && self.overlay != Some(!in_picture) {
            session.send(ViewerFrame::CursorOverlay(!in_picture));
            self.overlay = Some(!in_picture);
        }
    }

    fn show_held(&self, session: &ViewerSession, presenter: &dyn DirectPresenter) {
        let Some(stamp) = lock(&session.shared.direct.held).take() else {
            return;
        };
        if session.can_present(stamp.clone()) {
            match presenter.show_held(owner(&session.shared)) {
                Ok(true) => (session.presented_callback(stamp))(),
                Ok(false) => *lock(&session.shared.direct.held) = Some(stamp),
                Err(error) => eprintln!("Bildschirm: Direct3D 11: {error}"),
            }
        } else if session.shared.awaiting_floor(&stamp) {
            *lock(&session.shared.direct.held) = Some(stamp);
        }
    }

    /// Where the child goes this frame and whether it shows (it is placed
    /// and its swap chain sized before the first picture); `None` hides it.
    fn placement(
        &self,
        ctx: &egui::Context,
        area: Rect,
        dest: Option<Rect>,
        overlays: &[Rect],
        visible: bool,
    ) -> Option<(layout::DirectGeometry, bool)> {
        let dest = dest.filter(|_| visible)?;
        let exit = egui::Id::new("screen_fullscreen_exit");
        let (popup, layers) = ctx.memory(|memory| {
            let layers: Vec<(Order, bool, Rect)> = memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| layer.order != Order::Background)
                .filter_map(|layer| {
                    let rect = memory.area_rect(layer.id)?;
                    let cut_out = layer.id == exit || layer.order != Order::Foreground;
                    Some((layer.order, cut_out, rect))
                })
                .collect();
            (memory.any_popup_open(), layers)
        });
        let show = self.showing() && !layout::menu_over_picture(popup, &layers, area);
        let mut cut: Vec<Rect> = overlays.to_vec();
        cut.extend(layers.iter().map(|(_, _, rect)| *rect));
        let geometry = layout::geometry(area, dest, ctx.pixels_per_point(), &cut);
        Some((geometry, show))
    }
}
