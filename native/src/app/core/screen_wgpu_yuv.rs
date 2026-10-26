//! The GPU path of the desktop viewer (SCR-M7, decision C4, U25-5): the
//! decoder's planes go to the GPU as they are, `R8Unorm` for Y and either
//! `Rg8Unorm` (NV12) or two `R8Unorm` (I420) for the chroma, uploaded with
//! `queue.write_texture` (also on GL), and a full-screen triangle converts
//! them to RGB in the fragment shader: 3.11 MB instead of 8.29 MB per 1080p
//! picture, and no conversion on the CPU. The shader uses limited range and
//! the matrix of the stream; the CPU conversion of `pixel_convert` is the
//! reference (±2 per channel, `screen_task_viewer_yuv_shader_matches_cpu`).
//! Reconstruction integrates the texel footprint when reducing, uses Catmull-Rom
//! cubic convolution when enlarging, and exact luma texels at native size.
//! `install` puts the resources into the renderer; without them (no wgpu
//! surface, a shader the device refuses) the viewer draws BGRA on the CPU.
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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
static TEXTURE_LIMIT: AtomicU32 = AtomicU32::new(0);
/// Device capability, never a guessed constant; usable by frame negotiation.
pub(crate) fn texture_limit() -> Option<u32> {
    let limit = TEXTURE_LIMIT.load(Ordering::Relaxed);
    (limit > 0).then_some(limit)
}

/// A picture's planes as the GPU path takes them; made once per picture.
pub(crate) struct YuvFrame {
    /// Distinguishes pictures across sessions (upload once per picture).
    uid: u64,
    pub size: FrameSize,
    pub picture: YuvPicture,
}

impl YuvFrame {
    pub(crate) fn new(size: FrameSize, picture: YuvPicture) -> Self {
        Self {
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            size,
            picture,
        }
    }

    /// Whether the planes hold what `size` needs.
    pub(crate) fn fits(&self) -> bool {
        if crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(self.size).is_err() { return false; }
        let chroma = self.size.chroma();
        let plane = |data: &[u8], stride: usize, row: usize, rows: usize| {
            row > 0
                && stride >= row
                && stride.checked_mul(rows.saturating_sub(1)).and_then(|n| n.checked_add(row))
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

struct Planes {
    size: FrameSize,
    i420: bool,
    y: wgpu::Texture,
    u: wgpu::Texture,
    v: wgpu::Texture,
    /// Bind groups with the linear and with the nearest sampler.
    groups: [wgpu::BindGroup; 2],
    uploaded: u64,
}

/// Pipeline, sampler, parameters and plane textures; lives in the renderer's
/// callback resources.
pub(crate) struct YuvResources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    samplers: [wgpu::Sampler; 2],
    params: wgpu::Buffer,
    planes: Option<Planes>,
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

fn plane_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screen viewer plane"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_plane(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    stride: usize,
    (width, height): (u32, u32),
) {
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
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
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen viewer yuv"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            samplers: [
                sampler(wgpu::FilterMode::Linear),
                sampler(wgpu::FilterMode::Nearest),
            ],
            params,
            planes: None,
        }
    }

    /// Uploads `frame` unless it is already there.
    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &YuvFrame) {
        let limit = device.limits().max_texture_dimension_2d;
        if !frame.fits() || frame.size.width > limit || frame.size.height > limit {
            self.planes=None;
            return;
        }
        let picture = &frame.picture;
        let size = frame.size;
        let chroma = size.chroma();
        let i420 = matches!(picture.chroma, Chroma::I420 { .. });
        let current = self
            .planes
            .as_ref()
            .is_some_and(|planes| planes.size == size && planes.i420 == i420);
        if !current {
            self.planes = Some(self.new_planes(device, size, i420));
        }
        let Some(planes) = self.planes.as_mut() else {
            return;
        };
        if planes.uploaded == frame.uid {
            return;
        }
        planes.uploaded = frame.uid;
        write_plane(
            queue,
            &planes.y,
            &picture.y,
            picture.y_stride,
            (size.width, size.height),
        );
        match &picture.chroma {
            Chroma::Nv12 => write_plane(
                queue,
                &planes.u,
                &picture.uv,
                picture.uv_stride,
                (chroma.width, chroma.height),
            ),
            Chroma::I420 { u, v } => {
                let dims = (chroma.width, chroma.height);
                write_plane(queue, &planes.u, u, picture.uv_stride, dims);
                write_plane(queue, &planes.v, v, picture.uv_stride, dims);
            }
        }
        let [a, b, c, d] = coefficients(picture.matrix);
        let values = [a, b, c, d, if i420 { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0];
        let mut bytes = [0u8; 48];
        for (slot, value) in bytes.chunks_exact_mut(4).zip(values) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
        queue.write_buffer(&self.params, 0, &bytes);
    }

    fn new_planes(&self, device: &wgpu::Device, size: FrameSize, i420: bool) -> Planes {
        let chroma = size.chroma();
        let y = plane_texture(
            device,
            size.width,
            size.height,
            wgpu::TextureFormat::R8Unorm,
        );
        let (u, v) = if i420 {
            (
                plane_texture(
                    device,
                    chroma.width,
                    chroma.height,
                    wgpu::TextureFormat::R8Unorm,
                ),
                plane_texture(
                    device,
                    chroma.width,
                    chroma.height,
                    wgpu::TextureFormat::R8Unorm,
                ),
            )
        } else {
            (
                plane_texture(
                    device,
                    chroma.width,
                    chroma.height,
                    wgpu::TextureFormat::Rg8Unorm,
                ),
                // Unused with NV12; the layout still wants a texture.
                plane_texture(device, 1, 1, wgpu::TextureFormat::R8Unorm),
            )
        };
        let view =
            |texture: &wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (yv, uv, vv) = (view(&y), view(&u), view(&v));
        let group = |sampler: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("screen viewer yuv"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&yv),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&uv),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&vv),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            })
        };
        Planes {
            size,
            i420,
            groups: [group(&self.samplers[0]), group(&self.samplers[1])],
            y,
            u,
            v,
            uploaded: 0,
        }
    }

    fn set_view(&self, queue: &wgpu::Queue, rect: egui::Rect, screen: &egui_wgpu::ScreenDescriptor) {
        let Some(values) = super::screen_viewport::viewport_uv(rect,screen.pixels_per_point,screen.size_in_pixels) else { return; };
        let mut bytes = [0u8;16];
        for (slot,value) in bytes.chunks_exact_mut(4).zip(values) { slot.copy_from_slice(&value.to_ne_bytes()); }
        queue.write_buffer(&self.params,32,&bytes);
    }

    /// Draws the uploaded picture over the render pass's viewport.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, nearest: bool) {
        let Some(planes) = self.planes.as_ref() else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &planes.groups[usize::from(nearest)], &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The paint callback of one picture.
pub(crate) struct YuvCallback {
    pub frame: Arc<YuvFrame>,
    pub nearest: bool,
    pub rect: egui::Rect,
    pub presented: Option<super::screen_presented::Presented>,
}

impl egui_wgpu::CallbackTrait for YuvCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(yuv) = resources.get_mut::<YuvResources>() {
            yuv.upload(device, queue, &self.frame);
            yuv.set_view(queue,self.rect,screen);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(yuv) = resources.get::<YuvResources>() {
            if yuv.planes.as_ref().is_some_and(|planes| planes.uploaded == self.frame.uid) {
                yuv.draw(pass, self.nearest);
                if let Some(presented) = &self.presented { presented(); }
            }
        }
    }
}

/// Puts the resources into the renderer; `false` when the device refuses
/// the shader or the pipeline (the viewer then converts on the CPU).
pub(crate) fn install(state: &egui_wgpu::RenderState) -> bool {
    TEXTURE_LIMIT.store(state.device.limits().max_texture_dimension_2d, Ordering::Relaxed);
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

/// The paint callback shape for `rect`.
pub(crate) fn paint_callback(
    rect: egui::Rect,
    frame: Arc<YuvFrame>,
    nearest: bool,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, YuvCallback { frame, nearest, rect, presented: None })
}

/// Same YUV draw, with proof emitted only when these exact planes were drawn.
pub(crate) fn paint_presented(rect: egui::Rect, frame: Arc<YuvFrame>, nearest: bool,
    presented: super::screen_presented::Presented) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(rect, YuvCallback { frame, nearest, rect, presented: Some(presented) })
}

#[cfg(test)]
#[path = "screen_wgpu_yuv_tests.rs"]
mod tests;
