//! The start of the layer process (DSK-M7, U37-5, U37-6): the connection,
//! the required globals (`wl_compositor` version 4 or higher,
//! `zwlr_layer_shell_v1` version 3 or higher, `wl_seat`), the optional ones
//! (fractional scaling, `wl_data_device_manager` version 3, `wl_shm`,
//! cursor shape, xdg-output), the first views and the graphics. A missing
//! required global or missing graphics ends the process with exit code 1 and
//! one line that says why.
use std::sync::Arc;
use std::time::Instant;

use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::globals::ProvidesBoundGlobal;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
use smithay_client_toolkit::shm::Shm;
use wayland_client::globals::registry_queue_init;
use wayland_client::{EventQueue, Proxy};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

use super::clipboard::LayerClipboard;
use super::input::Input;
use super::registry::{build_outputs, connect, raw_output, OutputInfo};
use super::render::{Gpu, OutputRender};
use super::state::{LayerParams, LayerState, Scaling, Shared};
use super::view::{OutputView, ViewParts};
use crate::app::desktop_surface_clipboard;
use crate::desktop::core::config::ForeignDesktopIcons;
use crate::desktop::core::mechanism::{
    self, DisplayServer, LayerKeyboard, LayerNamespace, Mechanism,
};
use crate::desktop::os::{platform, Monitor};

/// Why the process cannot start: its exit code and the line for stderr.
pub(super) struct Failure {
    pub(super) code: i32,
    pub(super) line: String,
}

fn failure(line: impl Into<String>) -> Failure {
    Failure {
        code: 1,
        line: line.into(),
    }
}

fn missing(global: &str) -> Failure {
    failure(format!("Layer-Backend: {global} fehlt"))
}

/// Namespace and keyboard of the layer surfaces for the bound version of
/// the layer shell: the decision of the core (`mechanism::decide`).
fn layer_params(layer_version: u32) -> LayerParams {
    let mut facts = platform::session_facts();
    facts.display = DisplayServer::Wayland;
    facts.layer_shell_version = Some(layer_version);
    match mechanism::decide(&facts, ForeignDesktopIcons::Keep) {
        Mechanism::LayerShell {
            keyboard,
            namespace,
        } => LayerParams {
            namespace,
            keyboard_on_demand: keyboard == LayerKeyboard::OnDemand,
        },
        _ => LayerParams {
            namespace: LayerNamespace::SmartExplorer,
            keyboard_on_demand: layer_version >= 4,
        },
    }
}

/// Connects, binds the globals and opens a view for every output.
pub(super) fn start(mut shared: Shared) -> Result<(LayerState, EventQueue<LayerState>), Failure> {
    let conn = connect().map_err(|error| failure(format!("Layer-Backend: {error}")))?;
    let (globals, mut queue) = registry_queue_init::<LayerState>(&conn)
        .map_err(|error| failure(format!("Layer-Backend: Globale Objekte: {error}")))?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|_| missing("wl_compositor"))?;
    if compositor.wl_compositor().version() < 4 {
        return Err(missing("wl_compositor (Version 4 oder höher)"));
    }
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|_| missing("zwlr_layer_shell_v1"))?;
    let layer_version =
        <LayerShell as ProvidesBoundGlobal<ZwlrLayerShellV1, 1>>::bound_global(&layer_shell)
            .map(|shell| shell.version())
            .map_err(|_| missing("zwlr_layer_shell_v1"))?;
    if layer_version < 3 {
        return Err(missing("zwlr_layer_shell_v1 (Version 3 oder höher)"));
    }
    let seat_state = SeatState::new(&globals, &qh);
    if seat_state.seats().next().is_none() {
        return Err(missing("wl_seat"));
    }
    let scaling = match (
        globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ()),
        globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()),
    ) {
        (Ok(manager), Ok(viewporter)) => Some(Scaling {
            manager,
            viewporter,
        }),
        _ => None,
    };
    let manager = DataDeviceManagerState::bind(&globals, &qh)
        .ok()
        .filter(|manager| manager.data_device_manager().version() >= 3);
    let clipboard = manager.map(|manager| LayerClipboard::new(conn.clone(), qh.clone(), manager));
    shared.monitors.clear();
    let mut state = LayerState {
        conn: conn.clone(),
        qh: qh.clone(),
        registry: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        seat_state,
        shm: Shm::bind(&globals, &qh).ok(),
        scaling,
        params: layer_params(layer_version),
        gpu: None,
        views: Vec::new(),
        input: Input::default(),
        clipboard,
        shared,
        outputs_changed: false,
        next_view_id: 1,
        started: Instant::now(),
        exit: None,
    };
    // The outputs' events, then the seats' capabilities and devices.
    for _ in 0..2 {
        queue
            .roundtrip(&mut state)
            .map_err(|error| failure(format!("Layer-Backend: {error}")))?;
    }
    for seat in state.seat_state.seats().collect::<Vec<_>>() {
        state.attach_seat(&seat);
    }
    if let Some(clipboard) = state.clipboard.clone() {
        desktop_surface_clipboard::install(clipboard);
    }
    state.create_views()?;
    state.outputs_changed = false;
    Ok((state, queue))
}

impl LayerState {
    /// The outputs as the toolkit knows them, with the keys of the shell.
    fn current_outputs(&self) -> Vec<OutputInfo> {
        let raw = self
            .output_state
            .outputs()
            .filter_map(|output| self.output_state.info(&output))
            .map(|info| raw_output(&info))
            .collect();
        build_outputs(raw)
    }

    /// A surface, view and renderer for every output; the graphics start
    /// with the first surface (the adapter must be able to present on it).
    fn create_views(&mut self) -> Result<(), Failure> {
        let outputs = self.current_outputs();
        self.shared.monitors = outputs.iter().cloned().map(Monitor::from).collect();
        for info in outputs {
            let Some(wl_output) = self.output_state.outputs().find(|output| {
                self.output_state
                    .info(output)
                    .is_some_and(|i| i.id == info.global_name)
            }) else {
                continue;
            };
            let window = self.open_window(&wl_output, &info);
            let allow_alpha = self.params.namespace == LayerNamespace::SmartExplorer;
            let render = self.open_render(&window.surface, allow_alpha, &info)?;
            let id = self.next_view_id;
            self.next_view_id += 1;
            let view = {
                let parts = ViewParts {
                    sender: &self.shared.sender,
                    config: &self.shared.config,
                    folders: &self.shared.folders,
                    repaints: &self.shared.repaints,
                    ping: &self.shared.ping,
                    monitors: &self.shared.monitors,
                    clipboard: self.clipboard.clone().map(|clipboard| {
                        clipboard as Arc<dyn crate::app::desktop_surface_clipboard::FileClipboard>
                    }),
                };
                OutputView::new(id, window, info, Some(render), &parts)
            };
            self.views.push(view);
        }
        Ok(())
    }

    /// The wgpu surface of an output; the first one creates instance, adapter
    /// and device.
    fn open_render(
        &mut self,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        allow_alpha: bool,
        info: &OutputInfo,
    ) -> Result<OutputRender, Failure> {
        let wgpu_surface = match &self.gpu {
            Some(gpu) => gpu.surface_for(&self.conn, surface).map_err(|reason| {
                failure(format!("Layer-Backend: Fläche {}: {reason}", info.key))
            })?,
            None => {
                let (gpu, wgpu_surface) = Gpu::create(&self.conn, surface).map_err(|reason| {
                    failure(format!(
                        "Layer-Backend: keine Grafikbeschleunigung ({reason})"
                    ))
                })?;
                self.gpu = Some(gpu);
                wgpu_surface
            }
        };
        let Some(gpu) = &self.gpu else {
            return Err(failure("Layer-Backend: keine Grafikbeschleunigung"));
        };
        OutputRender::new(gpu, wgpu_surface, allow_alpha, (1, 1))
            .map_err(|reason| failure(format!("Layer-Backend: Fläche {}: {reason}", info.key)))
    }
}
