//! The output of the layer backend (DSK-M7, U37-8): wgpu on the raw handles
//! of the Wayland connection and of each `wl_surface`, and `egui_wgpu`'s
//! renderer without eframe. One instance, adapter and device per process;
//! one wgpu surface and one renderer per output (each output has its own
//! egui context and textures). There is no software fallback: without an
//! adapter the process says so and ends (`DESKTOP-LAYER-SOFTWARE`).
use std::ffi::c_void;
use std::future::Future;
use std::ptr::NonNull;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use eframe::egui;
use egui_wgpu::wgpu;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};

/// Suite switch (DSK-M15): `request_adapter` is skipped, so the process ends
/// as it does without graphics acceleration.
pub(super) const TEST_NO_ADAPTER: &str = "SE_DESKTOP_TEST_NO_ADAPTER";

/// Instance, adapter, device and queue of the process.
pub(super) struct Gpu {
    adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    instance: wgpu::Instance,
}

/// Runs a future to its end on this thread; wgpu's native futures are ready
/// at once.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// A wgpu surface on a `wl_surface` of this connection.
fn raw_surface(
    instance: &wgpu::Instance,
    connection: &Connection,
    surface: &WlSurface,
) -> Result<wgpu::Surface<'static>, String> {
    use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
    let display = NonNull::new(connection.backend().display_ptr().cast::<c_void>())
        .ok_or("wl_display ohne Zeiger")?;
    let window =
        NonNull::new(surface.id().as_ptr().cast::<c_void>()).ok_or("wl_surface ohne Zeiger")?;
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display)),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
    };
    // SAFETY: both pointers belong to the connection and the surface, which
    // outlive the wgpu surface (`OutputView` drops its renderer before its
    // window; the connection lives as long as the process loop).
    unsafe { instance.create_surface_unsafe(target) }.map_err(|error| error.to_string())
}

impl Gpu {
    /// Instance (the primary backends, Vulkan on Linux, and GL), the adapter
    /// that can present on `first` (low power: the surface is a desktop
    /// background) and its device. The error says why no acceleration is
    /// available.
    pub(super) fn create(
        connection: &Connection,
        first: &WlSurface,
    ) -> Result<(Gpu, wgpu::Surface<'static>), String> {
        if std::env::var(TEST_NO_ADAPTER).is_ok_and(|value| value == "1") {
            return Err(format!("Testschalter {TEST_NO_ADAPTER}"));
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..Default::default()
        });
        let surface = raw_surface(&instance, connection, first)?;
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .ok_or("kein Adapter für die Fläche")?;
        // The limits of egui_wgpu's default device, but never above what
        // the adapter offers.
        let base = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("se-desktop"),
                required_features: wgpu::Features::empty(),
                required_limits: base.using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .map_err(|error| error.to_string())?;
        Ok((
            Gpu {
                adapter,
                device,
                queue,
                instance,
            },
            surface,
        ))
    }

    /// The wgpu surface of a further output.
    pub(super) fn surface_for(
        &self,
        connection: &Connection,
        surface: &WlSurface,
    ) -> Result<wgpu::Surface<'static>, String> {
        raw_surface(&self.instance, connection, surface)
    }

    pub(super) fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }
}

/// What a frame ended with.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Drawn {
    Presented,
    /// The surface was lost or outdated and is configured again: draw again.
    Retry,
    /// No image this time (timeout): the next request draws.
    Skipped,
    /// The graphics memory is exhausted: the process ends.
    OutOfMemory,
}

/// The wgpu surface of one output with its egui renderer.
pub(super) struct OutputRender {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
    clear: wgpu::Color,
    /// The compositor blends the surface with what lies below (alpha mode).
    pub(super) transparent: bool,
}

impl OutputRender {
    /// Configures `surface` for `size` physical pixels. With `allow_alpha`
    /// (the namespace is not KDE's) a pre- or post-multiplied alpha mode is
    /// taken when the surface offers one; else the surface is opaque and the
    /// view paints the background itself.
    pub(super) fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        allow_alpha: bool,
        size: (u32, u32),
    ) -> Result<OutputRender, String> {
        let capabilities = surface.get_capabilities(&gpu.adapter);
        let format = egui_wgpu::preferred_framebuffer_format(&capabilities.formats)
            .map_err(|error| error.to_string())?;
        let alpha = allow_alpha
            .then(|| {
                [
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::PostMultiplied,
                ]
                .into_iter()
                .find(|mode| capabilities.alpha_modes.contains(mode))
            })
            .flatten();
        let (alpha_mode, transparent) = match alpha {
            Some(mode) => (mode, true),
            None if capabilities
                .alpha_modes
                .contains(&wgpu::CompositeAlphaMode::Opaque) =>
            {
                (wgpu::CompositeAlphaMode::Opaque, false)
            }
            None => (wgpu::CompositeAlphaMode::Auto, false),
        };
        let mut render = OutputRender {
            surface,
            config: wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: 1,
                height: 1,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 1,
                alpha_mode,
                view_formats: Vec::new(),
            },
            renderer: egui_wgpu::Renderer::new(&gpu.device, format, None, 1, false),
            clear: wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: if transparent { 0.0 } else { 1.0 },
            },
            transparent,
        };
        render.resize(gpu, size);
        Ok(render)
    }

    /// Configures the surface for a new size in physical pixels.
    pub(super) fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let limit = gpu.device.limits().max_texture_dimension_2d;
        self.config.width = size.0.clamp(1, limit);
        self.config.height = size.1.clamp(1, limit);
        self.surface.configure(&gpu.device, &self.config);
    }

    /// Paints one egui output. The texture changes are applied whatever
    /// happens to the image, or egui's font atlas would be lost. `before_present`
    /// runs just before the image is handed to the compositor (frame
    /// callback, scale).
    pub(super) fn draw(
        &mut self,
        gpu: &Gpu,
        ctx: &egui::Context,
        output: egui::FullOutput,
        before_present: impl FnOnce(),
    ) -> Drawn {
        let egui::FullOutput {
            shapes,
            textures_delta,
            pixels_per_point,
            ..
        } = output;
        let paint_jobs = ctx.tessellate(shapes, pixels_per_point);
        for (id, delta) in &textures_delta.set {
            self.renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let drawn = self.paint(gpu, &paint_jobs, pixels_per_point, before_present);
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
        drawn
    }

    fn paint(
        &mut self,
        gpu: &Gpu,
        paint_jobs: &[egui::ClippedPrimitive],
        pixels_per_point: f32,
        before_present: impl FnOnce(),
    ) -> Drawn {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&gpu.device, &self.config);
                return Drawn::Retry;
            }
            Err(wgpu::SurfaceError::Timeout) => return Drawn::Skipped,
            Err(wgpu::SurfaceError::OutOfMemory) => return Drawn::OutOfMemory,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("se-desktop"),
            });
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };
        let extra = self.renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            paint_jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("se-desktop"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.clear),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, paint_jobs, &screen);
        }
        gpu.queue
            .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        before_present();
        frame.present();
        Drawn::Presented
    }
}
