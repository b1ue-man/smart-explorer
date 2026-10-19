//! The state of the layer process (DSK-M7, U37-2): the Wayland globals, the
//! outputs with their views, the seats, the clipboard and the channel to the
//! shell. One `LayerState` is the data of the one event loop; the protocol
//! objects dispatch into it through the toolkit's delegates, and the handlers
//! of the toolkit's traits are here.
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use calloop::ping::Ping;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_data_device, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_touch, registry_handlers,
};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use super::clipboard::LayerClipboard;
use super::input::Input;
use super::registry::OutputScale;
use super::render::Gpu;
use super::view::{OutputView, RepaintQueue};
use crate::desktop::core::config::DesktopConfig;
use crate::desktop::core::mechanism::LayerNamespace;
use crate::desktop::core::protocol::{ShellMessage, SurfaceMessage};
use crate::desktop::os::feed_sources::DesktopFolder;
use crate::desktop::os::surface_link::FrameSender;
use crate::desktop::os::Monitor;

/// Fractional scaling needs both objects (U37-7); without either the
/// integer scale of the output applies.
pub(super) struct Scaling {
    pub(super) manager: WpFractionalScaleManagerV1,
    pub(super) viewporter: WpViewporter,
}

/// What the compositor's protocol versions and the desktop decide (C2.3).
pub(super) struct LayerParams {
    pub(super) namespace: LayerNamespace,
    /// Keyboard interactivity `on_demand` (layer shell version 4).
    pub(super) keyboard_on_demand: bool,
}

/// What the process shares with every output's view.
pub(super) struct Shared {
    pub(super) sender: Arc<FrameSender<SurfaceMessage>>,
    pub(super) inbox: Receiver<ShellMessage>,
    pub(super) send_alive: bool,
    pub(super) next_alive: Instant,
    pub(super) config: DesktopConfig,
    pub(super) folders: Vec<DesktopFolder>,
    pub(super) repaints: RepaintQueue,
    pub(super) ping: Ping,
    /// Every output of the session as the shell sees it.
    pub(super) monitors: Vec<Monitor>,
}

pub(super) struct LayerState {
    pub(super) conn: Connection,
    pub(super) qh: QueueHandle<LayerState>,
    pub(super) registry: RegistryState,
    pub(super) output_state: OutputState,
    pub(super) compositor: CompositorState,
    pub(super) layer_shell: LayerShell,
    pub(super) seat_state: SeatState,
    pub(super) shm: Option<Shm>,
    pub(super) scaling: Option<Scaling>,
    pub(super) params: LayerParams,
    pub(super) gpu: Option<Gpu>,
    pub(super) views: Vec<OutputView>,
    pub(super) input: Input,
    pub(super) clipboard: Option<Arc<LayerClipboard>>,
    pub(super) shared: Shared,
    /// An output came, went or changed: the shell hears of it once.
    pub(super) outputs_changed: bool,
    pub(super) next_view_id: u64,
    pub(super) started: Instant,
    /// Set when the process has to end: the exit code.
    pub(super) exit: Option<i32>,
}

impl LayerState {
    /// The surface changed its scale (the output's integer scale, or the
    /// compositor's preferred fraction); the renderer follows.
    pub(super) fn rescale(&mut self, surface: &WlSurface, scale: OutputScale, fractional: bool) {
        let Self { views, gpu, .. } = self;
        let Some(view) = views
            .iter_mut()
            .find(|view| &view.window.surface == surface)
        else {
            return;
        };
        if !view.window.set_scale(scale, fractional) {
            return;
        }
        if let (Some(render), Some(gpu)) = (view.render.as_mut(), gpu.as_ref()) {
            render.resize(gpu, view.window.pixels());
        }
        view.frame.dirty = true;
    }

    /// The process ends: the first reason counts.
    pub(super) fn end(&mut self, code: i32) {
        self.exit.get_or_insert(code);
    }
}

impl CompositorHandler for LayerState {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        self.rescale(surface, OutputScale::Integer(new_factor.max(1)), false);
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        if let Some(view) = self.view_for(surface) {
            view.frame.frame_pending = false;
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerState {
    /// The compositor closed the surface (its output went); the last one
    /// gone ends the process like a channel end: the shell starts it anew.
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.views.retain(|view| &view.window.layer != layer);
        self.outputs_changed = true;
        if self.views.is_empty() {
            self.end(crate::desktop::core::protocol::EXIT_CHANNEL_END);
        }
    }

    /// `ack_configure` is sent by the toolkit; the first configure lets the
    /// view draw.
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Self { views, gpu, .. } = self;
        let Some(view) = views.iter_mut().find(|view| &view.window.layer == layer) else {
            return;
        };
        let fallback = view.info.logical_size;
        if view.window.apply_configure(configure.new_size, fallback) {
            if let (Some(render), Some(gpu)) = (view.render.as_mut(), gpu.as_ref()) {
                render.resize(gpu, view.window.pixels());
            }
        }
        view.frame.dirty = true;
    }
}

impl OutputHandler for LayerState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.outputs_changed = true;
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.outputs_changed = true;
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.outputs_changed = true;
    }
}

impl ShmHandler for LayerState {
    fn shm_state(&mut self) -> &mut Shm {
        // Only a bound `wl_shm` sends events, and then `shm` is set.
        self.shm
            .as_mut()
            .expect("wl_shm sent an event without being bound")
    }
}

impl ProvidesRegistryState for LayerState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers!(OutputState, SeatState);
}

impl Dispatch<WpFractionalScaleV1, WlSurface> for LayerState {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        surface: &WlSurface,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.rescale(surface, OutputScale::Fraction(scale), true);
        }
    }
}

delegate_noop!(LayerState: WpFractionalScaleManagerV1);
delegate_noop!(LayerState: WpViewporter);
delegate_noop!(LayerState: WpViewport);
delegate_registry!(LayerState);
delegate_output!(LayerState);
delegate_compositor!(LayerState);
delegate_layer!(LayerState);
delegate_seat!(LayerState);
delegate_pointer!(LayerState);
delegate_touch!(LayerState);
delegate_shm!(LayerState);
delegate_data_device!(LayerState);
