//! How the desktop viewer presents a decoded picture (plan 2026-11-14
//! „Bildkette“ BK9: B-37 part 2, B-40) and the evidence in the locked crate
//! versions (eframe/egui-wgpu 0.29.1, wgpu 22.1.0, wgpu-core 22.1.0,
//! wgpu-hal 22.0.0, `native/Cargo.lock`).
//!
//! Window backend. eframe asks for wgpu without its default features
//! (`eframe-0.29.1/Cargo.toml:235-242`, `egui-wgpu-0.29.1/Cargo.toml:85-88`),
//! so wgpu's `dx12` feature (`wgpu-22.1.0/Cargo.toml:104`) is off and the
//! crate `d3d12` is not in the lock: the window draws with Vulkan on
//! Windows and Linux (`wgpu-22.1.0/Cargo.toml:162-166`), with wgpu's GL
//! backend where no Vulkan adapter exists (`:145-148`, `:212-215`; the
//! adapter order is Vulkan, Metal, DX12, GL, `wgpu-core-22.1.0/src/
//! instance.rs:918-930`), and with glow when wgpu gets no surface
//! (`screen_launch.rs`).
//!
//! Direct presentation (decoded surface without a read-back): not possible.
//! Importing a D3D11 texture or a VA-API DMA-BUF into a Vulkan device needs
//! `VK_KHR_external_memory_win32` resp. `VK_KHR_external_memory_fd` with
//! `VK_EXT_external_memory_dma_buf` enabled when the device is made; wgpu-hal
//! 22 enables no external-memory extension (`wgpu-hal-22.0.0/src/vulkan/
//! adapter.rs:855-1003`, no `external_memory` anywhere in `src/`), and
//! egui-wgpu 0.29.1 makes the device itself from a `DeviceDescriptor`
//! (`src/lib.rs:161-166`, `WgpuConfiguration` `:224-256` takes no device),
//! so no extension can be added. The GL fallbacks are not a way either: glow
//! prefers GLX/WGL over EGL (`eframe-0.29.1/src/native/glow_integration.rs:
//! 944`, `FallbackEgl`). Every hardware picture therefore takes the plane
//! path: one read-back in the decoder, one `write_texture` of the planes.
//! What would open the direct path: (a) wgpu's `dx12` feature (adds `d3d12`
//! 22.0.0 to the lock) with DX12 as the only Windows backend, then
//! `IDXGIResource1::CreateSharedHandle` on a shared NV12 copy of the
//! decoder texture, `ID3D12Device::OpenSharedHandle` on the device of
//! `Device::as_hal::<Dx12>` (`wgpu-hal-22.0.0/src/dx12/device.rs:312`),
//! `dx12::Device::texture_from_raw` (`:320`) with `TextureFormat::NV12`
//! (`dx12/adapter.rs:308`) and `wgpu::Device::create_texture_from_hal`
//! (`wgpu-22.1.0/src/lib.rs:3106`); (b) for Vulkan on both systems an eframe
//! that accepts a device the program made itself (assumption: eframe/
//! egui-wgpu 0.30 `WgpuSetup::Existing`, not in this registry) through
//! `vulkan::Adapter::device_from_raw` (`vulkan/adapter.rs:1593`) with the
//! external-memory extensions.
//!
//! Present mode (B-40). eframe keeps the surface configuration private
//! (`egui-wgpu-0.29.1/src/winit.rs:82` field `configuration`, `:148`
//! `configure_surface`; the app sees only `Frame::wgpu_render_state`,
//! `eframe-0.29.1/src/epi.rs:693`): „Niedrigste Latenz“ takes effect when
//! the window opens. It chooses `AutoNoVsync` (Immediate, else Mailbox,
//! else Fifo, `wgpu-types-22.0.0/src/lib.rs:5257-5260`): in full screen the
//! picture may tear (variable-refresh screens synchronise themselves), in a
//! composited window the compositor synchronises. Otherwise `AutoVsync`
//! with one frame queued (decision C4). `Mailbox` is never set: eframe does
//! not ask the surface first, and a surface without it ends the program.
use eframe::egui_wgpu::{self, wgpu};

/// The present mode of the viewer window.
pub fn present_mode(low_latency: bool) -> wgpu::PresentMode {
    if low_latency {
        wgpu::PresentMode::AutoNoVsync
    } else {
        wgpu::PresentMode::AutoVsync
    }
}

/// The wgpu settings of the viewer: at most one frame queued in either
/// mode; they take effect when the window opens.
pub fn wgpu_configuration(low_latency: bool) -> egui_wgpu::WgpuConfiguration {
    egui_wgpu::WgpuConfiguration {
        present_mode: present_mode(low_latency),
        desired_maximum_frame_latency: Some(1),
        ..Default::default()
    }
}

/// What draws the viewer window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowBackend {
    Vulkan,
    Dx12,
    /// wgpu's own GL backend.
    WgpuGl,
    /// eframe's glow renderer (no wgpu surface).
    Glow,
    Other,
}

impl WindowBackend {
    pub fn of_wgpu(backend: wgpu::Backend) -> Self {
        match backend {
            wgpu::Backend::Vulkan => Self::Vulkan,
            wgpu::Backend::Dx12 => Self::Dx12,
            wgpu::Backend::Gl => Self::WgpuGl,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vulkan => "Vulkan",
            Self::Dx12 => "DX12",
            Self::WgpuGl => "OpenGL (wgpu)",
            Self::Glow => "OpenGL (glow)",
            Self::Other => "andere",
        }
    }
}

/// Where the decoder leaves a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodedSurface {
    /// Planes or pixels in memory (software decoders, tiles).
    Memory,
    /// An NV12 texture of the Media Foundation decoder on D3D11 (BK5w).
    D3d11Texture,
    /// An NV12 surface of the VA-API decoder (BK4).
    VaSurface,
}

impl DecodedSurface {
    /// From the decoder's name, which names its stage (BK5w, BK4).
    pub fn of_decoder(name: &str) -> Self {
        if name.contains("D3D11VA") {
            Self::D3d11Texture
        } else if name.starts_with("vaapi-") {
            Self::VaSurface
        } else {
            Self::Memory
        }
    }
}

/// How a picture reaches the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentPath {
    /// The decoded surface is drawn as it is.
    Direct,
    /// The planes are uploaded (`write_texture`) and converted in the shader.
    PlaneUpload,
    /// BGRA pixels as a texture (tiles, glow, pictures the planes do not fit).
    BgraUpload,
}

impl PresentPath {
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "direkt",
            Self::PlaneUpload => "Upload YUV-Ebenen",
            Self::BgraUpload => "Upload BGRA",
        }
    }
}

/// Whether `backend` imports `surface` without a copy through the processor;
/// none does with the locked versions (module head).
pub fn imports(backend: WindowBackend, surface: DecodedSurface) -> bool {
    match (backend, surface) {
        (_, DecodedSurface::Memory) => false,
        // Vulkan: no external-memory extension on the device (wgpu-hal 22).
        (WindowBackend::Vulkan, _) => false,
        // DX12 is not compiled (wgpu without its `dx12` feature).
        (WindowBackend::Dx12, _) => false,
        // GL: glow prefers GLX/WGL, wgpu's GL backend has no import either.
        (WindowBackend::WgpuGl | WindowBackend::Glow | WindowBackend::Other, _) => false,
    }
}

/// The path of a picture: direct where the backend imports the surface and
/// no import failed in this session, else the planes when the window takes
/// them (`yuv`), else BGRA.
pub fn select(
    backend: WindowBackend,
    surface: DecodedSurface,
    yuv: bool,
    import_failed: bool,
) -> PresentPath {
    if !import_failed && imports(backend, surface) {
        PresentPath::Direct
    } else if yuv {
        PresentPath::PlaneUpload
    } else {
        PresentPath::BgraUpload
    }
}

/// The statistics line of the display path (B-40, 4.10a „Anzeige“).
pub fn stats_line(path: PresentPath, backend: WindowBackend, surface: DecodedSurface) -> String {
    let readback = match (path, surface) {
        (PresentPath::Direct, _) | (_, DecodedSurface::Memory) => "",
        _ => ", Hardwarebild zurückgelesen",
    };
    format!("Anzeige {} ({}{readback})", path.label(), backend.label())
}

/// The statistics line for the window of `ctx` and the session's decoder.
pub fn window_line(ctx: &eframe::egui::Context, decoder: Option<&str>, planes: bool) -> String {
    let backend = super::screen_render_owner::get(ctx)
        .map_or(WindowBackend::Other, |owner| owner.window_backend());
    let surface = DecodedSurface::of_decoder(decoder.unwrap_or_default());
    stats_line(select(backend, surface, planes, false), backend, surface)
}

/// The statistics lines of a session's view with its display line when a
/// picture is drawn (`planes`: `ViewWidget::planes_drawn`); the viewer and
/// the remote window (`family_viewer.rs`) show the same lines.
pub fn view_lines(
    ctx: &eframe::egui::Context,
    info: &super::screen_session::SessionInfo,
    planes: Option<bool>,
    unmapped_keys: u64,
) -> Vec<String> {
    let mut lines = super::screen_stats_overlay::stats_lines(info, unmapped_keys);
    lines.extend(planes.map(|planes| window_line(ctx, info.decoder.as_deref(), planes)));
    lines
}

#[cfg(test)]
#[path = "screen_present_tests.rs"]
mod tests;
