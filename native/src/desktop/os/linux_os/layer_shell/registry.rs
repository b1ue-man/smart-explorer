//! The outputs of the Wayland session (DSK-M7, U37-4): every `wl_output` with
//! its position and size in the compositor's logical coordinates, the key it
//! is known by (`layout/<key>.json`, the shell's monitor list) and its scale.
//! The shell asks `probe_outputs` on a connection of its own; the layer
//! process builds the same list from its own `OutputState`, so both sides use
//! the same keys. Wayland has no main monitor: the output at the top left
//! counts as primary (assumption, no effect on any right).
use smithay_client_toolkit::output::{OutputHandler, OutputInfo as WlOutputInfo, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::{delegate_output, delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_output;
use wayland_client::{Connection, QueueHandle};

use crate::desktop::os::Monitor;

/// Longest monitor key (`layout::MAX_MONITOR_KEY`).
const MAX_MONITOR_KEY: usize = 64;

/// The scale of an output or a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputScale {
    /// `wl_output.scale` / `wl_surface.set_buffer_scale`.
    Integer(i32),
    /// `wp_fractional_scale_v1.preferred_scale`: 120ths.
    Fraction(u32),
}

impl OutputScale {
    /// egui's `pixels_per_point`.
    pub(crate) fn pixels_per_point(self) -> f32 {
        match self {
            OutputScale::Integer(factor) => factor.max(1) as f32,
            OutputScale::Fraction(n) => n.max(1) as f32 / 120.0,
        }
    }

    /// Physical pixels of a logical size: `round(logical × scale)`.
    pub(crate) fn buffer_size(self, logical: (u32, u32)) -> (u32, u32) {
        match self {
            OutputScale::Integer(factor) => {
                let factor = factor.max(1) as u32;
                (
                    logical.0.saturating_mul(factor),
                    logical.1.saturating_mul(factor),
                )
            }
            OutputScale::Fraction(n) => (scaled_120(logical.0, n), scaled_120(logical.1, n)),
        }
    }

    /// Whether the buffer scale (`wl_surface.set_buffer_scale`) carries the
    /// scale; a fraction goes through `wp_viewport` instead.
    pub(crate) fn buffer_scale(self) -> i32 {
        match self {
            OutputScale::Integer(factor) => factor.max(1),
            OutputScale::Fraction(_) => 1,
        }
    }
}

fn scaled_120(value: u32, n: u32) -> u32 {
    if value == 0 {
        return 0;
    }
    let scaled = (u64::from(value) * u64::from(n.max(1)) + 60) / 120;
    u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
}

/// One output as the session reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputInfo {
    /// Name of the `wl_output` global (stable while the output exists).
    pub(crate) global_name: u32,
    pub(crate) key: String,
    pub(crate) logical_origin: [i32; 2],
    pub(crate) logical_size: [i32; 2],
    pub(crate) scale: OutputScale,
    pub(crate) primary: bool,
}

impl From<OutputInfo> for Monitor {
    /// `rect` is logical here (the layer session only, see
    /// `docs/oberflaechen/desktop-launcher.md`).
    fn from(output: OutputInfo) -> Monitor {
        let [x, y] = output.logical_origin;
        let [width, height] = output.logical_size;
        Monitor {
            key: output.key,
            primary: output.primary,
            rect: Some([x, y, x.saturating_add(width), y.saturating_add(height)]),
        }
    }
}

/// What the key of an output is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawOutput {
    pub(crate) global_name: u32,
    /// `wl_output.name` (version 4) or `zxdg_output_v1.name`.
    pub(crate) name: Option<String>,
    pub(crate) xdg_name: Option<String>,
    pub(crate) make: String,
    pub(crate) model: String,
    pub(crate) origin: [i32; 2],
    pub(crate) size: [i32; 2],
    pub(crate) scale_factor: i32,
}

/// Everything but ASCII letters, digits, `-`, `.`, `_` becomes `_`; the key
/// is a file name (`layout/<key>.json`) and never starts with a dot.
fn clean(text: &str) -> String {
    let mut cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.starts_with('.') {
        cleaned.replace_range(0..1, "_");
    }
    cleaned.truncate(MAX_MONITOR_KEY);
    cleaned
}

/// The key of an output (U37-4): the connector name of `wl_output` (version
/// 4), else of `zxdg_output_v1`, else `<make> <model>`, else
/// `output-<index>` (order of the global names).
pub(crate) fn output_key(
    name: Option<&str>,
    xdg_name: Option<&str>,
    make: &str,
    model: &str,
    index: usize,
) -> String {
    for candidate in [name, xdg_name].into_iter().flatten() {
        if !candidate.trim().is_empty() {
            return clean(candidate.trim());
        }
    }
    let label = format!("{} {}", make.trim(), model.trim());
    if !label.trim().is_empty() {
        return clean(label.trim());
    }
    clean(&format!("output-{index}"))
}

/// `base`, or `base-2`, `base-3`, … when `base` is taken already.
fn unique_key(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|key| key == base) {
        return base.to_string();
    }
    (2u32..)
        .map(|number| {
            let suffix = format!("-{number}");
            let mut key = base.to_string();
            key.truncate(MAX_MONITOR_KEY - suffix.len());
            key.push_str(&suffix);
            key
        })
        .find(|key| !taken.iter().any(|taken| taken == key))
        .unwrap_or_else(|| base.to_string())
}

/// The outputs with their keys, sorted by key. Duplicated keys get `-2`,
/// `-3` in the order of the global names; the primary output is the one
/// with the smallest origin by (y, x).
pub(crate) fn build_outputs(mut raw: Vec<RawOutput>) -> Vec<OutputInfo> {
    raw.sort_by_key(|output| output.global_name);
    let mut taken: Vec<String> = Vec::new();
    let mut outputs: Vec<OutputInfo> = Vec::new();
    for (index, output) in raw.iter().enumerate() {
        let base = output_key(
            output.name.as_deref(),
            output.xdg_name.as_deref(),
            &output.make,
            &output.model,
            index,
        );
        let key = unique_key(&base, &taken);
        taken.push(key.clone());
        outputs.push(OutputInfo {
            global_name: output.global_name,
            key,
            logical_origin: output.origin,
            logical_size: output.size,
            scale: OutputScale::Integer(output.scale_factor.max(1)),
            primary: false,
        });
    }
    let primary = outputs
        .iter()
        .enumerate()
        .min_by_key(|(_, output)| (output.logical_origin[1], output.logical_origin[0]))
        .map(|(index, _)| index);
    if let Some(index) = primary {
        outputs[index].primary = true;
    }
    outputs.sort_by(|left, right| left.key.cmp(&right.key));
    outputs
}

/// The raw facts of SCTK's output information: the logical position and size
/// of `zxdg_output_v1`, else the position of `wl_output.geometry` and the
/// current mode divided by the scale.
pub(crate) fn raw_output(info: &WlOutputInfo) -> RawOutput {
    let scale_factor = info.scale_factor.max(1);
    let size = info.logical_size.unwrap_or_else(|| {
        info.modes
            .iter()
            .find(|mode| mode.current)
            .map_or((0, 0), |mode| {
                (
                    mode.dimensions.0 / scale_factor,
                    mode.dimensions.1 / scale_factor,
                )
            })
    });
    let origin = info.logical_position.unwrap_or(info.location);
    RawOutput {
        global_name: info.id,
        name: info.name.clone(),
        xdg_name: None,
        make: info.make.clone(),
        model: info.model.clone(),
        origin: [origin.0, origin.1],
        size: [size.0, size.1],
        scale_factor,
    }
}

/// Whether `libwayland-client` can be loaded; the backend loads it at run
/// time (R78), and without it the Wayland connection would panic.
pub(crate) fn wayland_client_available() -> bool {
    // SAFETY: loading the library runs its initialisers only; nothing of it
    // is called.
    unsafe { libloading::Library::new("libwayland-client.so.0") }.is_ok()
}

/// A connection to the compositor of this session.
pub(crate) fn connect() -> Result<Connection, String> {
    if !wayland_client_available() {
        return Err("libwayland-client.so.0 fehlt".into());
    }
    Connection::connect_to_env().map_err(|error| error.to_string())
}

struct Probe {
    registry: RegistryState,
    outputs: OutputState,
}

impl OutputHandler for Probe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for Probe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers!(OutputState);
}

delegate_registry!(Probe);
delegate_output!(Probe);

/// Asks the compositor for its outputs on a connection of its own: two
/// round trips (globals, then the events of the bound outputs), sorted by
/// key. The shell calls this once a second.
pub(crate) fn probe_outputs() -> Result<Vec<OutputInfo>, String> {
    let connection = connect()?;
    let (globals, mut queue) =
        registry_queue_init::<Probe>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let mut probe = Probe {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
    };
    for _ in 0..2 {
        queue
            .roundtrip(&mut probe)
            .map_err(|error| error.to_string())?;
    }
    let raw = probe
        .outputs
        .outputs()
        .filter_map(|output| probe.outputs.info(&output))
        .map(|info| raw_output(&info))
        .collect();
    Ok(build_outputs(raw))
}
