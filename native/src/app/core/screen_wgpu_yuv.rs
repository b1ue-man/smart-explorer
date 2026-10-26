//! The GPU path of the desktop viewer (SCR-M7, decision C4, U25-5): the
//! decoder's planes go to the GPU as they are, `R8Unorm` for Y and either
//! `Rg8Unorm` (NV12) or two `R8Unorm` (I420) for the chroma, uploaded with
//! `queue.write_texture`, and a full-screen triangle converts
//! them to RGB in the fragment shader: 3.11 MB instead of 8.29 MB per 1080p
//! picture, and no conversion on the CPU. The shader uses limited range and
//! the matrix of the stream; the CPU conversion of `pixel_convert` is the
//! reference (±2 per channel, `screen_task_viewer_yuv_shader_matches_cpu`).
//! Reconstruction integrates the texel footprint when reducing, uses Catmull-Rom
//! cubic convolution when enlarging, and exact luma texels at native size.
//! `install` puts the resources into the renderer; without them (no wgpu
//! surface, a shader the device refuses) the viewer draws BGRA on the CPU.
use super::screen_render_owner::{lock, Backend, Native, RenderSlot};
use super::screen_render_upload::{Generation, UploadTicket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};

use crate::screen::core::host_backend::{Chroma, YuvPicture};
use crate::screen::core::pixel_convert::FrameSize;

/// The shader (`screen_wgpu_yuv.wgsl`): a full-screen triangle and the
/// conversion of the planes to RGB.
const SHADER: &str = include_str!("screen_wgpu_yuv.wgsl");

pub(crate) use super::screen_scaling::coefficients;
#[cfg(test)]
pub(crate) use super::screen_scaling::yuv_to_rgb;

static NEXT_UID: AtomicU64 = AtomicU64::new(1);
/// A picture's planes as the GPU path takes them; made once per picture.
pub(crate) struct YuvFrame {
    /// Distinguishes pictures across sessions (upload once per picture).
    pub(super) uid: u64,
    pub size: FrameSize,
    pub picture: YuvPicture,
    pub memory: Option<Arc<crate::screen::core::frame_memory::MemoryLease>>,
}

impl YuvFrame {
    pub(crate) fn new(size: FrameSize, picture: YuvPicture) -> Self {
        Self {
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            size,
            picture,
            memory: None,
        }
    }

    /// Whether the planes hold what `size` needs.
    pub(crate) fn fits(&self) -> bool {
        if crate::screen::core::frame_budget::FrameLimits::RESOURCE
            .check(self.size)
            .is_err()
        {
            return false;
        }
        let chroma = self.size.chroma();
        let plane = |data: &[u8], stride: usize, row: usize, rows: usize| {
            row > 0
                && stride >= row
                && stride
                    .checked_mul(rows.saturating_sub(1))
                    .and_then(|n| n.checked_add(row))
                    .is_some_and(|needed| data.len() >= needed)
        };
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let (cw, ch) = (chroma.width as usize, chroma.height as usize);
        let picture = &self.picture;
        let luma = plane(&picture.y, picture.y_stride, w, h);
        luma && match &picture.chroma {
            Chroma::Nv12 => plane(&picture.uv, picture.uv_stride, cw * 2, ch),
            Chroma::I420 { u, v } => {
                plane(u, picture.uv_stride, cw, ch) && plane(v, picture.uv_stride, cw, ch)
            }
        }
    }
}

/// Shared immutable pipeline state; image-dependent storage belongs to a slot.
pub(crate) struct YuvResources {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) samplers: [wgpu::Sampler; 2],
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl YuvResources {
    pub(crate) fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen viewer yuv"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("screen viewer yuv"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen viewer yuv"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen viewer yuv"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = |filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("screen viewer yuv"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            pipeline,
            layout,
            samplers: [
                sampler(wgpu::FilterMode::Linear),
                sampler(wgpu::FilterMode::Nearest),
            ],
        }
    }
}

/// A callback can draw only the exact still-active resident selected by the slot.
struct YuvCallback {
    slot: std::sync::Arc<super::screen_render_owner::RenderSlot>,
    generation: super::screen_render_upload::Generation,
    stamp: super::screen_presented::PresentedFrame,
    nearest: bool,
    rect: egui::Rect,
    owner: std::sync::Weak<super::screen_render_owner::RendererOwner>,
    prepared: std::sync::atomic::AtomicBool,
    presented: Option<super::screen_presented::Presented>,
}
impl egui_wgpu::CallbackTrait for YuvCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let ready = self.owner.upgrade().is_some_and(|owner| {
            owner.prepare_yuv(&self.slot, &self.generation, &self.stamp, self.rect, screen)
        });
        self.prepared.store(ready, Ordering::Release);
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if !self.prepared.load(Ordering::Acquire) {
            return;
        }
        let state = super::screen_render_owner::lock(&self.slot.state);
        let Some(resident) = state.resident.as_ref().filter(|r| {
            r.package.active()
                && r.package.generation().same(&self.generation)
                && r.package.frame().as_ref() == Some(&self.stamp)
        }) else {
            return;
        };
        if let (super::screen_render_owner::Native::Yuv(image), Some(resources)) =
            (&resident.native, resources.get::<YuvResources>())
        {
            image.draw(resources, pass, self.nearest);
            if let Some(receipt) = &self.presented {
                receipt();
            }
        }
    }
}
/// Puts the resources into the renderer; `false` when the device refuses
/// the shader or the pipeline (the viewer then converts on the CPU).
pub(crate) fn install(state: &egui_wgpu::RenderState) -> bool {
    state.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let resources = YuvResources::new(&state.device, state.target_format);
    let refused = tokio::runtime::Builder::new_current_thread()
        .build()
        .map(|runtime| runtime.block_on(state.device.pop_error_scope()))
        .unwrap_or(None);
    if refused.is_some() {
        return false;
    }
    let _ = state.renderer.write().callback_resources.insert(resources);
    true
}

/// The slot owns the upload and selects the stamp whose planes will actually draw.
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    frame: Arc<YuvFrame>,
    stamp: super::screen_presented::PresentedFrame,
    nearest: bool,
    slot: &Arc<super::screen_render_owner::RenderSlot>,
    receipt: impl Fn(
        super::screen_presented::PresentedFrame,
    ) -> Option<super::screen_presented::Presented>,
) -> bool {
    let ctx = painter.ctx();
    let screen = ctx.screen_rect().size() * ctx.pixels_per_point();
    let generation = slot.generation();
    slot.queue(super::screen_render_upload::Package::Yuv {
        frame,
        stamp,
        generation,
        rect,
        screen: [
            screen.x.round().max(1.0) as u32,
            screen.y.round().max(1.0) as u32,
        ],
        ppp: ctx.pixels_per_point(),
    });
    let Some(owner) = super::screen_render_owner::get(ctx) else {
        return false;
    };
    owner.attach(slot);
    let state = super::screen_render_owner::lock(&slot.state);
    let Some(resident) = state.resident.as_ref().filter(|r| {
        r.package.active() && matches!(r.native, super::screen_render_owner::Native::Yuv(_))
    }) else {
        return false;
    };
    let Some(stamp) = resident.package.frame() else {
        return false;
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        YuvCallback {
            slot: slot.clone(),
            generation: resident.package.generation().clone(),
            presented: receipt(stamp.clone()),
            stamp,
            nearest,
            rect,
            owner: Arc::downgrade(&owner),
            prepared: false.into(),
        },
    ));
    true
}
#[cfg(test)]
#[path = "screen_wgpu_yuv_tests.rs"]
mod tests;

impl super::screen_render_owner::RendererOwner {
    /// Called under eframe's callback-resource lock; this method never locks its renderer.
    pub(crate) fn prepare_yuv(
        &self,
        slot: &RenderSlot,
        generation: &Generation,
        stamp: &super::screen_presented::PresentedFrame,
        rect: egui::Rect,
        screen: &egui_wgpu::ScreenDescriptor,
    ) -> bool {
        let backend = lock(&self.backend);
        let Backend::Wgpu(b) = &*backend else {
            return false;
        };
        if self.closed() || !b.available() {
            return false;
        }
        let mut s = lock(&slot.state);
        let Some(r) = s.resident.as_mut().filter(|r| {
            r.package.active()
                && r.package.generation().same(generation)
                && r.package.frame().as_ref() == Some(stamp)
        }) else {
            return false;
        };
        let Native::Yuv(image) = &mut r.native else {
            return false;
        };
        if !image.matches_view(rect, screen.pixels_per_point, screen.size_in_pixels) {
            if !r.upload.drain.confirmed() {
                return false;
            }
            image.set_view(
                &b.state.queue,
                rect,
                screen.pixels_per_point,
                screen.size_in_pixels,
            );
            r.upload = UploadTicket {
                drain: b.submit(),
                package: r.package.clone(),
                staging: None,
            };
        }
        true
    }
}
