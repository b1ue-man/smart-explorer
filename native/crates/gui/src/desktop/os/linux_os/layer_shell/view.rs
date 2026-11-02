//! One output of the layer process (DSK-M7, U37-2): its own egui context,
//! its view model, state and actions, its surface and renderer. The view
//! (`show_surface`) is the one the eframe windows draw; only the host, the
//! input and the paint differ.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calloop::ping::Ping;
use eframe::egui::{self, Event, Modifiers, Pos2, RawInput, Rect, ViewportId};

use super::host::LayerHost;
use super::registry::OutputInfo;
use super::render::OutputRender;
use super::surface::LayerWindow;
use crate::app::desktop_surface_actions::SystemActions;
use crate::app::desktop_surface_clipboard::FileClipboard;
use crate::app::desktop_surface_grid_ui::{show_surface, SurfaceState};
use crate::app::desktop_surface_view_model::{Outbox, ViewModel, ALIVE_EVERY};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::protocol::SurfaceMessage;
use crate::desktop::os::feed_sources::DesktopFolder;
use crate::desktop::os::surface_link::FrameSender;
use crate::desktop::os::Monitor;

/// Repaint requests of the egui contexts (view id, delay), taken by the loop.
pub(super) type RepaintQueue = Arc<Mutex<Vec<(u64, Duration)>>>;

/// The one channel to the shell, shared by every output's view model; an
/// arrangement change leaves it with the key of its output.
pub(super) struct SharedOutbox {
    pub(super) sender: Arc<FrameSender<SurfaceMessage>>,
    pub(super) key: String,
}

impl Outbox for SharedOutbox {
    fn send(&self, message: SurfaceMessage) -> bool {
        let message = match message {
            SurfaceMessage::Layout { op, monitor: None } => SurfaceMessage::Layout {
                op,
                monitor: Some(self.key.clone()),
            },
            other => other,
        };
        self.sender.send(message)
    }
}

/// What an output keeps between frames.
#[derive(Default)]
pub(super) struct FrameState {
    pub(super) events: Vec<Event>,
    /// Something changed (input, model, size, scale, a repaint request).
    pub(super) dirty: bool,
    /// A frame callback is outstanding: nothing is drawn before it comes.
    pub(super) frame_pending: bool,
    /// egui asked for a frame at this time.
    pub(super) redraw_at: Option<Instant>,
    /// The seat's keyboard focus is on this surface.
    pub(super) focused: bool,
    /// The pointer is over this surface (its cursor is the view's to set).
    pub(super) pointer_inside: bool,
}

pub(super) struct OutputView {
    pub(super) id: u64,
    // The wgpu surface goes before the window it draws on.
    pub(super) render: Option<OutputRender>,
    pub(super) window: LayerWindow,
    pub(super) info: OutputInfo,
    pub(super) ctx: egui::Context,
    pub(super) view: ViewModel,
    pub(super) ui: SurfaceState,
    pub(super) actions: SystemActions,
    pub(super) host: LayerHost,
    pub(super) frame: FrameState,
}

/// Everything a new output's view needs from the process.
pub(super) struct ViewParts<'a> {
    pub(super) sender: &'a Arc<FrameSender<SurfaceMessage>>,
    pub(super) config: &'a DesktopConfig,
    pub(super) folders: &'a [DesktopFolder],
    pub(super) repaints: &'a RepaintQueue,
    pub(super) ping: &'a Ping,
    /// Every output of the session, for the host.
    pub(super) monitors: &'a [Monitor],
    pub(super) clipboard: Option<Arc<dyn FileClipboard>>,
}

impl OutputView {
    /// The context asks the loop for frames through `repaints` and `ping`
    /// (a model arrived, an action finished, an animation runs).
    pub(super) fn new(
        id: u64,
        window: LayerWindow,
        info: OutputInfo,
        render: Option<OutputRender>,
        parts: &ViewParts<'_>,
    ) -> OutputView {
        let ctx = egui::Context::default();
        let repaints = parts.repaints.clone();
        let ping = parts.ping.clone();
        ctx.set_request_repaint_callback(move |request| {
            if let Ok(mut queue) = repaints.lock() {
                queue.push((id, request.delay));
            }
            ping.ping();
        });
        let transparent = render.as_ref().is_some_and(|render| render.transparent);
        // The heartbeat is the process's (`LayerState::heartbeat`), not a view's.
        let view = ViewModel::new(
            &info.key,
            Box::new(SharedOutbox {
                sender: parts.sender.clone(),
                key: info.key.clone(),
            }),
            false,
            parts.config.clone(),
            parts.folders.to_vec(),
        );
        let host = LayerHost::new(
            &info.key,
            parts.monitors.to_vec(),
            info.logical_origin,
            ctx.clone(),
            parts.clipboard.clone(),
        );
        OutputView {
            id,
            render,
            window,
            info,
            ui: SurfaceState::new(transparent),
            actions: SystemActions::new(ctx.clone()),
            ctx,
            view,
            host,
            frame: FrameState {
                dirty: true,
                ..FrameState::default()
            },
        }
    }

    /// Queues input events for the next frame.
    pub(super) fn push(&mut self, events: impl IntoIterator<Item = Event>) {
        self.frame.events.extend(events);
        self.frame.dirty = true;
    }

    /// May draw now: configured, a renderer, and no frame callback pending.
    pub(super) fn wants_draw(&self, now: Instant) -> bool {
        self.window.configured
            && self.render.is_some()
            && !self.frame.frame_pending
            && self.window.size.0 > 0
            && self.window.size.1 > 0
            && (self.frame.dirty || self.frame.redraw_at.is_some_and(|at| at <= now))
    }

    /// The input of the next frame: the surface in points, the scale, the
    /// events and modifiers collected since the last frame.
    pub(super) fn raw_input(
        &mut self,
        started: Instant,
        modifiers: Modifiers,
        max_texture_side: usize,
    ) -> RawInput {
        let (width, height) = self.window.size;
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32));
        let mut raw = RawInput {
            screen_rect: Some(screen),
            max_texture_side: Some(max_texture_side),
            time: Some(started.elapsed().as_secs_f64()),
            modifiers,
            events: std::mem::take(&mut self.frame.events),
            focused: self.frame.focused,
            ..RawInput::default()
        };
        if let Some(info) = raw.viewports.get_mut(&ViewportId::ROOT) {
            info.title = Some("se Desktop".into());
            info.native_pixels_per_point = Some(self.window.scale.pixels_per_point());
            info.inner_rect = Some(screen);
            info.focused = Some(self.frame.focused);
        }
        raw
    }

    /// One frame of the view; it asks for another at the latest in a second.
    pub(super) fn run_ui(&mut self, raw: RawInput) -> egui::FullOutput {
        let ctx = self.ctx.clone();
        let Self {
            view,
            ui,
            actions,
            host,
            ..
        } = self;
        ctx.run(raw, |ctx| {
            show_surface(ctx, view, ui, &*host, actions);
            ctx.request_repaint_after(ALIVE_EVERY);
        })
    }
}
