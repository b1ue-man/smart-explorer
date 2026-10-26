//! `screen_task_viewer_yuv_shader_matches_cpu` (SCR-M7, U25-5): the shader's
//! conversion of decoded planes to RGB differs from the CPU conversion of
//! `pixel_convert` by at most 2 per channel, for both matrices and both
//! chroma layouts. The arithmetic (`yuv_to_rgb`) is checked everywhere; the
//! shader itself needs a wgpu adapter (a software one such as lavapipe is
//! enough): without one the test says so, and with `SE_SCREEN_TASK_GPU=1`
//! (the GUI stage of the suite) a missing adapter fails it.
use eframe::egui_wgpu::wgpu;

use super::*;
use crate::screen::core::pixel_convert::{i420_to_bgra, BgraImage, I420Image, YuvMatrix};

const SIDE: u32 = 64;
const TOLERANCE: i32 = 2;

/// Pseudo-random bytes in `low..=high`.
struct Noise(u32);

impl Noise {
    fn next(&mut self, low: u8, high: u8) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        low + (self.0 % u32::from(high - low + 1)) as u8
    }
}

fn close(a: u8, b: u8) -> bool {
    (i32::from(a) - i32::from(b)).abs() <= TOLERANCE
}

#[test]
fn screen_task_viewer_yuv_shader_matches_cpu() {
    arithmetic_matches_the_cpu_conversion();
    match gpu() {
        Some((device, queue)) => shader_matches_the_cpu_conversion(&device, &queue),
        None => {
            assert_ne!(
                std::env::var("SE_SCREEN_TASK_GPU").as_deref(),
                Ok("1"),
                "no wgpu adapter on a runner that has to have one"
            );
            eprintln!("nicht ausgeführt: no wgpu adapter, the shader itself was not compared");
        }
    }
}

/// The shader's arithmetic against the fixed-point conversion, over a
/// lattice of the legal sample range.
fn arithmetic_matches_the_cpu_conversion() {
    let size = FrameSize::new(2, 2);
    for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
        for y in (16..=235u8).step_by(9) {
            for u in (16..=240u8).step_by(14) {
                for v in (16..=240u8).step_by(14) {
                    let planes = I420Image {
                        size,
                        y: vec![y; 4],
                        u: vec![u],
                        v: vec![v],
                    };
                    let mut cpu = BgraImage::new(size).unwrap();
                    i420_to_bgra(&planes, matrix, &mut cpu).unwrap();
                    let pixel = &cpu.data()[..4];
                    let shader = yuv_to_rgb(y, u, v, matrix);
                    assert!(
                        close(pixel[2], shader[0])
                            && close(pixel[1], shader[1])
                            && close(pixel[0], shader[2]),
                        "{matrix:?} y={y} u={u} v={v}: cpu {:?} shader {shader:?}",
                        [pixel[2], pixel[1], pixel[0]]
                    );
                }
            }
        }
    }
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().ok()?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    for fallback in [false, true] {
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: fallback,
            compatible_surface: None,
        };
        let Some(adapter) = runtime.block_on(instance.request_adapter(&options)) else {
            continue;
        };
        let device =
            runtime.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None));
        if let Ok(pair) = device {
            return Some(pair);
        }
    }
    None
}

/// Draws the uploaded picture over a 64 × 64 target and reads it back (RGBA).
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &mut YuvResources,
    frame: &YuvFrame,
) -> Vec<u8> {
    let mut image = super::super::screen_render_yuv::YuvImage::new(device);
    image.upload(resources, device, queue, frame);
    let extent = wgpu::Extent3d {
        width: SIDE,
        height: SIDE,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("yuv test target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("yuv test readback"),
        size: u64::from(SIDE * SIDE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("yuv test"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        image.draw(resources, &mut pass, true);
    }
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver
        .recv()
        .expect("the readback answers")
        .expect("the readback maps");
    let data = slice.get_mapped_range().to_vec();
    readback.unmap();
    data
}

fn shader_matches_the_cpu_conversion(device: &wgpu::Device, queue: &wgpu::Queue) {
    let mut resources = YuvResources::new(device, wgpu::TextureFormat::Rgba8Unorm);
    let size = FrameSize::new(SIDE, SIDE);
    let chroma = size.chroma();
    let mut noise = Noise(0x1234_5678);
    for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
        for nv12 in [false, true] {
            let planes = I420Image {
                size,
                y: (0..size.pixels()).map(|_| noise.next(16, 235)).collect(),
                u: (0..chroma.pixels()).map(|_| noise.next(16, 240)).collect(),
                v: (0..chroma.pixels()).map(|_| noise.next(16, 240)).collect(),
            };
            let mut cpu = BgraImage::new(size).unwrap();
            i420_to_bgra(&planes, matrix, &mut cpu).unwrap();
            let picture = if nv12 {
                let interleaved: Vec<u8> = planes
                    .u
                    .iter()
                    .zip(&planes.v)
                    .flat_map(|(u, v)| [*u, *v])
                    .collect();
                YuvPicture {
                    y: planes.y.clone(),
                    y_stride: SIDE as usize,
                    uv: interleaved,
                    uv_stride: SIDE as usize,
                    chroma: Chroma::Nv12,
                    matrix,
                }
            } else {
                YuvPicture {
                    y: planes.y.clone(),
                    y_stride: SIDE as usize,
                    uv: Vec::new(),
                    uv_stride: chroma.width as usize,
                    chroma: Chroma::I420 {
                        u: planes.u.clone(),
                        v: planes.v.clone(),
                    },
                    matrix,
                }
            };
            let frame = YuvFrame::new(size, picture);
            assert!(frame.fits());
            let gpu = render(device, queue, &mut resources, &frame);
            let mut worst = 0;
            for (index, pixel) in gpu.chunks_exact(4).enumerate() {
                let reference = &cpu.data()[index * 4..index * 4 + 4];
                for (shader, cpu) in [
                    (pixel[0], reference[2]),
                    (pixel[1], reference[1]),
                    (pixel[2], reference[0]),
                ] {
                    worst = worst.max((i32::from(shader) - i32::from(cpu)).abs());
                }
            }
            assert!(
                worst <= TOLERANCE,
                "{matrix:?} nv12={nv12}: worst difference {worst}"
            );
        }
    }
}
