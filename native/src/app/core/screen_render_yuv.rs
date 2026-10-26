//! Slot-owned YUV textures, bind groups and parameters; shared pipeline stays immutable.
use eframe::{egui,egui_wgpu::{self,wgpu}};
use super::screen_wgpu_yuv::{YuvFrame,YuvResources,coefficients};
use crate::screen::core::{pixel_convert::FrameSize,host_backend::Chroma};
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

pub(crate) struct YuvImage { params:wgpu::Buffer, planes:Option<Planes>,view:Option<[f32;4]> }
impl YuvImage {
    pub(crate) fn new(device:&wgpu::Device)->Self {
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen viewer yuv"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {params,planes:None,view:None}
    }
    pub(crate) fn compatible(&self,frame:&YuvFrame)->bool {self.planes.as_ref().is_some_and(|p|p.size==frame.size&&p.i420==matches!(frame.picture.chroma,Chroma::I420{..}))}
    pub(crate) fn destroy(&self) {if let Some(p)=&self.planes {p.y.destroy();p.u.destroy();p.v.destroy();}self.params.destroy();}
    /// Uploads `frame` unless it is already there.
    pub(crate) fn upload(&mut self, resources:&YuvResources, device: &wgpu::Device, queue: &wgpu::Queue, frame: &YuvFrame) {
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
            self.planes = Some(self.new_planes(resources, device, size, i420));
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

    fn new_planes(&self, resources:&YuvResources, device: &wgpu::Device, size: FrameSize, i420: bool) -> Planes {
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
                layout: &resources.layout,
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
            groups: [group(&resources.samplers[0]), group(&resources.samplers[1])],
            y,
            u,
            v,
            uploaded: 0,
        }
    }

    pub(crate) fn set_view(&mut self, queue: &wgpu::Queue, rect: egui::Rect, ppp:f32, screen:[u32;2]) {
        let Some(values) = super::screen_viewport::viewport_uv(rect,ppp,screen) else { return; };
        self.view=Some(values);
        let mut bytes = [0u8;16];
        for (slot,value) in bytes.chunks_exact_mut(4).zip(values) { slot.copy_from_slice(&value.to_ne_bytes()); }
        queue.write_buffer(&self.params,32,&bytes);
    }

    pub(crate) fn matches_view(&self,rect:egui::Rect,ppp:f32,screen:[u32;2])->bool {
        self.view==super::screen_viewport::viewport_uv(rect,ppp,screen)
    }
    /// Draws the uploaded picture over the render pass's viewport.
    pub(crate) fn draw(&self, resources:&YuvResources, pass: &mut wgpu::RenderPass<'_>, nearest: bool) {
        let Some(planes) = self.planes.as_ref() else {
            return;
        };
        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &planes.groups[usize::from(nearest)], &[]);
        pass.draw(0..3, 0..1);
    }
}
