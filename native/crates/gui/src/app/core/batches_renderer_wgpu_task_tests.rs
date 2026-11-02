//! Real headless WGPU: owned registration, upload without Surface, draw and retirement.
use super::*;
use eframe::egui_wgpu::wgpu;
fn state() -> Option<egui_wgpu::RenderState> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().ok()?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    for fallback in [false, true] {
        let Some(adapter) =
            runtime.block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: fallback,
                ..Default::default()
            }))
        else {
            continue;
        };
        let Ok((device, queue)) =
            runtime.block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
        else {
            continue;
        };
        let renderer =
            egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, None, 1, true);
        return Some(egui_wgpu::RenderState {
            adapter: Arc::new(adapter),
            available_adapters: Arc::from([]),
            device: Arc::new(device),
            queue: Arc::new(queue),
            target_format: wgpu::TextureFormat::Rgba8Unorm,
            renderer: Arc::new(egui::epaint::mutex::RwLock::new(renderer)),
        });
    }
    None
}
fn missing(fixture: &str) {
    eprintln!("SE_TASK_UNVERIFIED_JSON={{\"fixture\":\"{fixture}\",\"reason\":\"WGPU adapter unavailable\"}}");
    assert_ne!(
        std::env::var("SE_SCREEN_TASK_GPU").as_deref(),
        Ok("1"),
        "real WGPU backend required by this runner"
    );
    eprintln!("Nicht ausgeführt: realer WGPU-Adapter fehlt; kein Beleg für Upload ohne Surface und Retirement nach Draw.");
}
fn owner(ctx: &egui::Context, state: egui_wgpu::RenderState) -> Arc<RendererOwner> {
    let limit = state.device.limits().max_texture_dimension_2d;
    let owner = Arc::new(RendererOwner {
        backend: Mutex::new(Backend::Wgpu(WgpuBackend::new(state, ctx))),
        slots: Mutex::new(Vec::new()),
        closed: false.into(),
        limit,
        yuv: false,
    });
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(KEY), Arc::downgrade(&owner)));
    owner
}
fn draw(state: &egui_wgpu::RenderState, id: egui::TextureId) {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2.0, 2.0));
    let mut mesh = egui::Mesh::with_texture(id);
    mesh.add_rect_with_uv(
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    let jobs = [egui::ClippedPrimitive {
        clip_rect: rect,
        primitive: egui::epaint::Primitive::Mesh(mesh),
    }];
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [2, 2],
        pixels_per_point: 1.0,
    };
    let target = state.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("owned screen test target"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: state.target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = state.device.create_command_encoder(&Default::default());
    let extra = state.renderer.write().update_buffers(
        &state.device,
        &state.queue,
        &mut encoder,
        &jobs,
        &screen,
    );
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("owned screen last draw"),
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
        state
            .renderer
            .read()
            .render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    state
        .queue
        .submit(extra.into_iter().chain([encoder.finish()]));
}
#[test]
fn batches_task_b6_renderer_wgpu_upload_without_surface_and_retire_after_later_draw() {
    let Some(state) = state() else {
        missing("batches_task_b6_renderer_wgpu_upload_without_surface_and_retire_after_later_draw");
        return;
    };
    let ctx = egui::Context::default();
    let owner = owner(&ctx, state.clone());
    let pool = Arc::new(MemoryPool::default());
    let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
    let mut texture = texture(&ctx, Some(memory.clone()));
    drop(memory);
    owner.service(&ctx);
    let id = texture.id().unwrap();
    assert!(state.renderer.read().texture(&id).is_some());
    state.device.poll(wgpu::Maintain::Wait);
    assert!(lock(&texture.slot().state)
        .resident
        .as_ref()
        .unwrap()
        .upload
        .drain
        .confirmed());
    // No Surface has been created. This later render pass is behind the upload proof.
    draw(&state, id);
    texture.reset();
    owner.after_update(&ctx);
    assert!(state.renderer.read().texture(&id).is_none());
    assert!(lock(&texture.slot().state).retiring.is_some());
    assert!(pool.reserve(None, 1024 * 1024).is_err());
    state.device.poll(wgpu::Maintain::Wait);
    owner.after_update(&ctx);
    assert!(lock(&texture.slot().state).retiring.is_none());
    assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
    owner.shutdown(&ctx, None);
    owner.returned();
    // The same real backend path now carries distinct local upload/surface claims.
    use super::super::super::screen_image_memory::{owner_limits, CpuPixels, ImageAccess};
    use crate::screen::core::frame_memory::CapacityCeiling;
    use crate::screen::core::image_admission::{ImageAdmission, ImageRole, LocalImagePolicy};
    let admission = ImageAdmission::in_test_pool(
        pool,
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(MAX_WORKING_BYTES),
        },
        owner_limits().unwrap(),
    );
    let login_owner = self::owner(&ctx, state.clone());
    let slot = Arc::new(RenderSlot::default());
    login_owner.attach(&slot);
    let source = CpuPixels::convert(&image(), ImageAccess::Local(admission.clone())).unwrap();
    slot.queue(Package::Cpu(
        super::super::super::screen_render_upload::PreparedImage {
            image: source,
            memory: None,
            frame: Some(stamp(1)),
            generation: slot.generation(),
            seq: 1,
        },
    ));
    login_owner.service(&ctx);
    let id = slot.id().unwrap();
    assert_eq!(admission.live_count(ImageRole::UploadStaging).unwrap(), 1);
    assert_eq!(admission.live_count(ImageRole::RenderSurface).unwrap(), 1);
    state.device.poll(wgpu::Maintain::Wait);
    login_owner.after_update(&ctx);
    assert_eq!(admission.live_count(ImageRole::UploadStaging).unwrap(), 0);
    assert_eq!(admission.live_count(ImageRole::RenderSurface).unwrap(), 1);
    draw(&state, id);
    slot.reset();
    login_owner.after_update(&ctx);
    assert_eq!(admission.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert!(state.renderer.read().texture(&id).is_none());
    state.device.poll(wgpu::Maintain::Wait);
    login_owner.after_update(&ctx);
    assert_eq!(admission.live_bytes().unwrap(), 0);
    login_owner.shutdown(&ctx, None);
    login_owner.returned();
}
#[test]
fn batches_task_b6_renderer_wgpu_yuv_planes_leave_slot_on_cpu_fallback() {
    let Some(state) = state() else {
        missing("batches_task_b6_renderer_wgpu_yuv_planes_leave_slot_on_cpu_fallback");
        return;
    };
    assert!(super::super::super::screen_wgpu_yuv::install(&state));
    let ctx = egui::Context::default();
    let owner = owner(&ctx, state.clone());
    let slot = Arc::new(RenderSlot::default());
    owner.attach(&slot);
    let pool = Arc::new(MemoryPool::default());
    let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
    let mut frame = super::super::super::screen_wgpu_yuv::YuvFrame::new(
        FrameSize::new(2, 2),
        crate::screen::core::host_backend::YuvPicture {
            y: vec![16; 4],
            y_stride: 2,
            uv: vec![128; 2],
            uv_stride: 2,
            chroma: crate::screen::core::host_backend::Chroma::Nv12,
            matrix: crate::screen::core::pixel_convert::YuvMatrix::Bt709,
        },
    );
    frame.memory = Some(memory.clone());
    drop(memory);
    slot.queue(Package::Yuv {
        frame: Arc::new(frame),
        stamp: stamp(1),
        generation: slot.generation(),
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2.0, 2.0)),
        screen: [2, 2],
        ppp: 1.0,
    });
    owner.service(&ctx);
    assert!(matches!(
        &lock(&slot.state).resident.as_ref().unwrap().native,
        Native::Yuv(_)
    ));
    state.device.poll(wgpu::Maintain::Wait);
    slot.reset();
    owner.after_update(&ctx);
    assert!(lock(&slot.state).resident.is_none());
    assert!(pool.reserve(None, 1024 * 1024).is_err());
    state.device.poll(wgpu::Maintain::Wait);
    owner.after_update(&ctx);
    assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
    assert!(state
        .renderer
        .read()
        .callback_resources
        .get::<super::super::super::screen_wgpu_yuv::YuvResources>()
        .is_some());
    owner.shutdown(&ctx, None);
    owner.returned();
}
