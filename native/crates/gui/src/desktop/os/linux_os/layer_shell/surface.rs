//! The layer surface of one output (DSK-M7, U37-6, U37-7): layer `bottom`,
//! anchored to all four edges without a size of its own, no exclusive zone,
//! keyboard `on_demand` when the protocol version has it. The compositor
//! answers with the size of the output's work area (bars with an exclusive
//! zone shrink it), so the surface is the work area itself. The scale is the
//! fractional one (`wp_fractional_scale_v1` with `wp_viewport`) or the
//! integer scale of the output (`wl_surface.set_buffer_scale`).
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerSurface,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use super::registry::{OutputInfo, OutputScale};
use super::state::LayerState;

/// The surface of one output with its size and scale.
pub(super) struct LayerWindow {
    pub(super) layer: LayerSurface,
    pub(super) surface: WlSurface,
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    /// Logical size from the last `configure`; zero before the first one.
    pub(super) size: (u32, u32),
    pub(super) scale: OutputScale,
    /// The first `configure` was acknowledged; nothing is drawn before.
    pub(super) configured: bool,
}

impl LayerState {
    /// Creates the layer surface for `output` and commits it empty, so the
    /// compositor answers with the first `configure`.
    pub(super) fn open_window(&self, output: &WlOutput, info: &OutputInfo) -> LayerWindow {
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Bottom,
            Some(self.params.namespace.as_str()),
            Some(output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(0);
        layer.set_margin(0, 0, 0, 0);
        layer.set_keyboard_interactivity(if self.params.keyboard_on_demand {
            KeyboardInteractivity::OnDemand
        } else {
            KeyboardInteractivity::None
        });
        let (viewport, fractional) = match &self.scaling {
            Some(scaling) => (
                Some(scaling.viewporter.get_viewport(&surface, &self.qh, ())),
                Some(
                    scaling
                        .manager
                        .get_fractional_scale(&surface, &self.qh, surface.clone()),
                ),
            ),
            None => (None, None),
        };
        layer.commit();
        LayerWindow {
            layer,
            surface,
            viewport,
            fractional,
            size: (0, 0),
            scale: info.scale,
            configured: false,
        }
    }
}

impl LayerWindow {
    /// Takes the size of a `configure`; a zero side means "any" and falls
    /// back to the output's logical size. `true` when the size changed.
    pub(super) fn apply_configure(&mut self, new_size: (u32, u32), fallback: [i32; 2]) -> bool {
        let pick = |given: u32, fallback: i32| {
            if given == 0 {
                u32::try_from(fallback).unwrap_or(0)
            } else {
                given
            }
        };
        let size = (pick(new_size.0, fallback[0]), pick(new_size.1, fallback[1]));
        let changed = size != self.size || !self.configured;
        self.size = size;
        self.configured = true;
        changed
    }

    /// A new scale; `true` when it differs. A fractional scale stays in
    /// force against later integer changes of the output.
    pub(super) fn set_scale(&mut self, scale: OutputScale, fractional: bool) -> bool {
        if matches!(self.scale, OutputScale::Fraction(_)) && !fractional {
            return false;
        }
        let changed = self.scale != scale;
        self.scale = scale;
        changed
    }

    /// Size in physical pixels.
    pub(super) fn pixels(&self) -> (u32, u32) {
        self.scale.buffer_size(self.size)
    }

    /// Hands the compositor the scale of the next buffer; just before every
    /// present (the requests apply with the commit of the presentation).
    pub(super) fn apply_scale(&self) {
        let destination = match (&self.viewport, self.scale) {
            (Some(_), OutputScale::Fraction(_)) => (self.size.0 as i32, self.size.1 as i32),
            _ => (-1, -1),
        };
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(destination.0, destination.1);
        }
        self.surface.set_buffer_scale(self.scale.buffer_scale());
    }
}

impl Drop for LayerWindow {
    fn drop(&mut self) {
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}
