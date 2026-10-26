//! Native resources; egui owns only their user-ID registration.
use super::screen_render_owner::{set_resident, Native};
use super::screen_render_upload::{Drain, Package, PreparedImage, SlotState};
use super::screen_render_yuv::YuvImage;
use eframe::{
    egui,
    egui_wgpu::{self, wgpu},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex, PoisonError,
};

struct PollState {
    pending: Option<wgpu::SubmissionIndex>,
    stopped: bool,
}
pub(crate) struct WgpuBackend {
    pub state: egui_wgpu::RenderState,
    poll: Option<Arc<(Mutex<PollState>, Condvar)>>,
    pub lost: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl WgpuBackend {
    pub(crate) fn new(state: egui_wgpu::RenderState, ctx: &egui::Context) -> Self {
        let pending = Arc::new((
            Mutex::new(PollState {
                pending: None,
                stopped: false,
            }),
            Condvar::new(),
        ));
        let signal = pending.clone();
        let device = state.device.clone();
        let repaint = ctx.clone();
        let worker = std::thread::Builder::new()
            .name("screen-gpu-drain".into())
            .spawn(move || loop {
                let index = {
                    let mut s = signal.0.lock().unwrap_or_else(PoisonError::into_inner);
                    while !s.stopped && s.pending.is_none() {
                        s = signal.1.wait(s).unwrap_or_else(PoisonError::into_inner);
                    }
                    if s.stopped {
                        return;
                    }
                    s.pending.take()
                };
                if let Some(index) = index {
                    device.poll(wgpu::Maintain::WaitForSubmissionIndex(index));
                    repaint.request_repaint();
                }
            })
            .ok();
        let poll = worker.as_ref().map(|_| pending);
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        let repaint = ctx.clone();
        state.device.set_device_lost_callback(move |_, _| {
            flag.store(true, Ordering::Release);
            super::screen_render_owner::notify_all();
            repaint.request_repaint();
        });
        Self {
            state,
            poll,
            lost,
            worker,
        }
    }
    pub(crate) fn available(&self) -> bool {
        self.poll.is_some() && !self.lost.load(Ordering::Acquire)
    }
    pub(crate) fn submit(&self) -> Drain {
        let drain = Drain::default();
        let done = drain.clone();
        let index = self.state.queue.submit([]);
        self.state
            .queue
            .on_submitted_work_done(move || done.confirm());
        if let Some(poll) = &self.poll {
            // Queue order lets one latest index subsume older pending indices.
            poll.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pending = Some(index);
            poll.1.notify_one();
        }
        drain
    }
    pub(crate) fn upload(&self, s: &mut SlotState, package: Package) {
        let Ok(claims) = s.claim_upload(&package) else {
            s.candidate = Some(package);
            return;
        };
        let native = match &package {
            Package::Cpu(image) => {
                let native = match s.resident.take().map(|r| r.native) {
                    Some(Native::Wgpu(t)) => t,
                    _ => CpuImage::new(self, image),
                };
                native.upload(self, image);
                Native::Wgpu(native)
            }
            Package::Yuv {
                frame,
                rect,
                screen,
                ppp,
                ..
            } => {
                let renderer = self.state.renderer.read();
                let Some(resources) = renderer
                    .callback_resources
                    .get::<super::screen_wgpu_yuv::YuvResources>()
                else {
                    s.candidate = Some(package);
                    return;
                };
                let mut native = match s.resident.take().map(|r| r.native) {
                    Some(Native::Yuv(t)) => t,
                    _ => YuvImage::new(&self.state.device),
                };
                native.upload(resources, &self.state.device, &self.state.queue, frame);
                native.set_view(&self.state.queue, *rect, *ppp, *screen);
                Native::Yuv(native)
            }
        };
        let drain = self.submit();
        set_resident(s, native, package, drain, claims);
    }
    pub(crate) fn wait(&self) {
        self.state.device.poll(wgpu::Maintain::Wait);
    }
}
pub(crate) struct CpuImage {
    pub texture: wgpu::Texture,
    pub id: egui::TextureId,
    pub size: [usize; 2],
}
impl CpuImage {
    pub(crate) fn new(backend: &WgpuBackend, image: &PreparedImage) -> Self {
        let size = image.image.size;
        let state = &backend.state;
        let texture = state.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen owned RGBA"),
            size: wgpu::Extent3d {
                width: size[0] as u32,
                height: size[1] as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = state.renderer.write().register_native_texture(
            &state.device,
            &view,
            wgpu::FilterMode::Nearest,
        );
        Self { texture, id, size }
    }
    pub(crate) fn upload(&self, backend: &WgpuBackend, image: &PreparedImage) {
        // Color32 is the pinned egui representation (RGBA bytes), no full-image copy.
        backend.state.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.image.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(self.size[0] as u32 * 4),
                rows_per_image: Some(self.size[1] as u32),
            },
            wgpu::Extent3d {
                width: self.size[0] as u32,
                height: self.size[1] as u32,
                depth_or_array_layers: 1,
            },
        );
    }
    pub(crate) fn retire(&self, backend: &WgpuBackend) {
        backend.state.renderer.write().free_texture(&self.id);
        self.texture.destroy();
    }
}

impl Drop for WgpuBackend {
    fn drop(&mut self) {
        if let Some(poll) = self.poll.take() {
            poll.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stopped = true;
            poll.1.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
