//! One renderer instance owns bounded slots until actual backend retirement.
use super::screen_image_memory::GpuClaims;
use super::screen_render_glow::{GlowBackend, GlowImage};
pub(crate) use super::screen_render_upload::RenderSlot;
use super::screen_render_upload::{Drain, Package, SlotState, UploadTicket};
use super::screen_render_wgpu::{CpuImage, WgpuBackend};
use super::screen_render_yuv::YuvImage;
use crate::screen::core::image_admission::ImageHold;
use eframe::egui;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
fn events() -> &'static (Mutex<u64>, Condvar) {
    static EVENTS: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    EVENTS.get_or_init(|| (Mutex::new(0), Condvar::new()))
}
pub(crate) fn notify_all() {
    let (m, c) = events();
    let mut epoch = lock(m);
    *epoch = epoch.wrapping_add(1);
    c.notify_all();
}
pub(crate) fn wait_event(epoch: u64) {
    let (m, c) = events();
    let mut current = lock(m);
    while *current == epoch {
        current = c.wait(current).unwrap_or_else(PoisonError::into_inner);
    }
}
pub(crate) fn event_epoch() -> u64 {
    *lock(&events().0)
}

pub(super) enum Native {
    Wgpu(CpuImage),
    Glow(GlowImage),
    Yuv(YuvImage),
    #[cfg(test)]
    Controlled(egui::TextureId, [usize; 2]),
}
pub(super) struct Resident {
    pub package: Package,
    pub native: Native,
    pub upload: UploadTicket,
    pub surface: Option<ImageHold>,
}
pub(super) struct Retirement {
    pub(super) package: Package,
    pub(super) upload: UploadTicket,
    pub(super) drain: Drain,
    pub(super) surface: Option<ImageHold>,
    pub(super) superseded: Option<Package>,
}
pub(super) enum Backend {
    Wgpu(WgpuBackend),
    Glow(GlowBackend),
    Unavailable,
    #[cfg(test)]
    Controlled {
        upload: bool,
    },
}
pub(crate) struct RendererOwner {
    pub(super) backend: Mutex<Backend>,
    slots: Mutex<Vec<Arc<RenderSlot>>>,
    closed: std::sync::atomic::AtomicBool,
    limit: u32,
    yuv: bool,
}
const KEY: &str = "screen_renderer_owner";
pub(crate) fn get(ctx: &egui::Context) -> Option<Arc<RendererOwner>> {
    ctx.data(|d| d.get_temp::<Weak<RendererOwner>>(egui::Id::new(KEY)))
        .and_then(|w| w.upgrade())
}
impl RendererOwner {
    pub(crate) fn install(
        cc: &eframe::CreationContext<'_>,
        shader: Option<eframe::egui_glow::ShaderVersion>,
        dithering: bool,
    ) -> Arc<Self> {
        let (backend, limit, yuv) = if let Some(state) = &cc.wgpu_render_state {
            let yuv = super::screen_wgpu_yuv::install(state);
            let limit = state.device.limits().max_texture_dimension_2d;
            (
                Backend::Wgpu(WgpuBackend::new(state.clone(), &cc.egui_ctx)),
                limit,
                yuv,
            )
        } else if let Some(gl) = &cc.gl {
            match GlowBackend::new(gl.clone(), cc.get_proc_address, shader, dithering) {
                Ok(backend) => {
                    let limit = backend.texture_limit();
                    (Backend::Glow(backend), limit, false)
                }
                Err(error) => {
                    eprintln!("Bildschirmrenderer: {error}");
                    (Backend::Unavailable, 0, false)
                }
            }
        } else {
            (Backend::Unavailable, 0, false)
        };
        let owner = Arc::new(Self {
            backend: Mutex::new(backend),
            slots: Mutex::new(Vec::new()),
            closed: false.into(),
            limit,
            yuv,
        });
        cc.egui_ctx
            .data_mut(|d| d.insert_temp(egui::Id::new(KEY), Arc::downgrade(&owner)));
        owner
    }
    pub(crate) fn texture_limit(&self) -> Option<u32> {
        (self.limit > 0).then_some(self.limit)
    }
    pub(crate) fn available(&self) -> bool {
        !self.closed()
            && match &*lock(&self.backend) {
                Backend::Wgpu(b) => b.available(),
                Backend::Glow(b) => b.available(),
                #[cfg(test)]
                Backend::Controlled { .. } => true,
                _ => false,
            }
    }
    pub(crate) fn yuv(&self) -> bool {
        self.yuv && self.available()
    }
    pub(crate) fn attach(&self, slot: &Arc<RenderSlot>) {
        let mut slots = lock(&self.slots);
        if !slots.iter().any(|s| Arc::ptr_eq(s, slot)) {
            slots.push(slot.clone());
        }
    }
    pub(crate) fn admission_pending(&self) -> bool {
        lock(&self.slots).iter().any(|slot| {
            let s = lock(&slot.state);
            s.retiring.is_some()
                || s.resident.as_ref().is_some_and(|r| !r.package.active())
                || s.admission.as_ref().is_some_and(|w| w.strong_count() > 0)
        })
    }
    pub(crate) fn closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }
    /// GL quarantine invalidates every presentation backed by that native context.
    /// Call outside a slot lock; login invalidation never acquires renderer locks.
    pub(super) fn quarantine_slots(&self) {
        for slot in lock(&self.slots).iter() {
            lock(&slot.state).quarantine();
        }
        notify_all();
    }
    /// Called on the GUI thread before inner update. No render pass is outstanding here.
    pub(crate) fn service(&self, ctx: &egui::Context) {
        self.service_mode(ctx, true);
    }
    pub(crate) fn after_update(&self, ctx: &egui::Context) {
        self.service_mode(ctx, false);
    }
    fn service_mode(&self, ctx: &egui::Context, upload: bool) {
        let mut backend = lock(&self.backend);
        if let Backend::Glow(gl) = &mut *backend {
            gl.service();
        }
        let mut slots = lock(&self.slots);
        let quarantined = matches!(&*backend,Backend::Glow(gl) if !gl.available());
        if quarantined {
            for slot in slots.iter() {
                lock(&slot.state).quarantine();
            }
        }
        for slot in slots.iter() {
            let mut state = lock(&slot.state);
            Self::step(
                &mut backend,
                &mut state,
                upload && !self.closed() && !quarantined,
            );
            if !quarantined && matches!(&*backend,Backend::Glow(gl) if !gl.available()) {
                break;
            }
        }
        // Retirement can itself discover a broken context after the initial service.
        if !quarantined && matches!(&*backend,Backend::Glow(gl) if !gl.available()) {
            for slot in slots.iter() {
                lock(&slot.state).quarantine();
            }
        }
        slots.retain(|slot| {
            let s = lock(&slot.state);
            !s.closed
                || s.resident.is_some()
                || s.retiring.is_some()
                || s.admission.as_ref().is_some_and(|w| w.strong_count() > 0)
        });
        if slots.iter().any(|slot| {
            let s = lock(&slot.state);
            s.retiring.is_some()
                || s.candidate.is_some()
                || s.resident
                    .as_ref()
                    .is_some_and(|r| !r.upload.drain.confirmed())
        }) {
            ctx.request_repaint();
        }
        drop(slots);
        drop(backend);
    }
    fn step(backend: &mut Backend, s: &mut SlotState, allow_upload: bool) {
        if s.retiring
            .as_ref()
            .is_some_and(|r| r.drain.confirmed() && r.upload.drain.confirmed())
        {
            s.retiring = None;
            notify_all();
        }
        if let Some(resident) = &mut s.resident {
            if resident.upload.drain.confirmed() {
                resident.upload.staging = None;
            }
        }
        if let Some(retiring) = &mut s.retiring {
            if retiring.upload.drain.confirmed() {
                retiring.upload.staging = None;
            }
        }
        let retire = s.resident.as_ref().is_some_and(|r| {
            !r.package.active()
                || s.closed
                || s.candidate
                    .as_ref()
                    .is_some_and(|p| !compatible(&r.native, p))
        });
        if retire && s.retiring.is_none() {
            if !s.closed && s.resident.as_ref().is_some_and(|r| r.package.active()) {
                if let Some(candidate) = s.candidate.as_ref().filter(|candidate| candidate.active())
                {
                    if s.next_upload.is_none() {
                        let Ok(claims) = candidate.upload_claims(None) else {
                            return;
                        };
                        s.next_upload = Some(claims);
                    }
                }
            }
            if let Some(r) = s.resident.take() {
                s.admission = r.package.memory().as_ref().map(Arc::downgrade);
                let drain = match (&mut *backend, &r.native) {
                    (Backend::Wgpu(b), Native::Wgpu(t)) => {
                        t.retire(b);
                        b.submit()
                    }
                    (Backend::Wgpu(b), Native::Yuv(t)) => {
                        t.destroy();
                        b.submit()
                    }
                    (Backend::Glow(b), Native::Glow(image)) => b.retire(image.id),
                    #[cfg(test)]
                    (Backend::Controlled { .. }, Native::Controlled(..)) => Drain::default(),
                    _ => Drain::default(),
                };
                // This fresh submission/fence follows the LAST render pass, not only upload.
                s.retiring = Some(Retirement {
                    package: r.package,
                    upload: r.upload,
                    drain,
                    surface: r.surface,
                    superseded: None,
                });
            }
        }
        if !allow_upload
            || s.closed
            || s.retiring.is_some()
            || s.resident
                .as_ref()
                .is_some_and(|r| !r.upload.drain.confirmed())
        {
            return;
        }
        let Some(package) = s.candidate.take().filter(|p| p.active()) else {
            return;
        };
        if s.resident
            .as_ref()
            .is_some_and(|r| r.package.same_image(&package))
        {
            return;
        }
        if let Backend::Wgpu(b) = backend {
            if !b.available() {
                s.candidate = Some(package);
                return;
            }
            b.upload(s, package);
        } else {
            #[cfg(test)]
            if let Backend::Controlled { upload: true } = backend {
                let Ok(claims) = s.claim_upload(&package) else {
                    s.candidate = Some(package);
                    return;
                };
                set_resident(
                    s,
                    Native::Controlled(
                        egui::TextureId::User(1),
                        match &package {
                            Package::Cpu(p) => p.image.size,
                            _ => [0, 0],
                        },
                    ),
                    package,
                    Drain::default(),
                    claims,
                );
                return;
            }
            s.candidate = Some(package);
        }
    }
    pub(crate) fn shutdown(&self, ctx: &egui::Context, gl: Option<&eframe::glow::Context>) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        for slot in lock(&self.slots).iter() {
            slot.close();
        }
        if let Backend::Glow(backend) = &*lock(&self.backend) {
            if !gl.is_some_and(|gl| backend.context_matches(gl)) {
                return;
            }
        }
        self.service(ctx);
        let mut backend = lock(&self.backend);
        match &mut *backend {
            Backend::Wgpu(b) => b.wait(),
            Backend::Glow(b) => b.shutdown(),
            _ => {}
        }
        drop(backend);
        self.service(ctx);
    }
    /// run_native has dropped its concrete native integration before this call.
    pub(crate) fn returned(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        let mut backend = std::mem::replace(&mut *lock(&self.backend), Backend::Unavailable);
        let slots = lock(&self.slots);
        for slot in slots.iter() {
            slot.close();
            if matches!(backend, Backend::Wgpu(_)) {
                Self::step(&mut backend, &mut lock(&slot.state), false);
            }
        }
        if let Backend::Wgpu(b) = &backend {
            b.wait();
        }
        // Drop all app-native references and stop/join its one polling service first.
        // Glow has no valid context here; concrete native-instance return is its proof.
        drop(backend);
        for slot in slots.iter() {
            let mut s = lock(&slot.state);
            s.candidate = None;
            s.next_upload = None;
            s.resident = None;
            s.retiring = None;
            s.admission = None;
        }
        drop(slots);
        lock(&self.slots).clear();
        notify_all();
    }
}
fn compatible(native: &Native, package: &Package) -> bool {
    match (native, package) {
        (Native::Wgpu(t), Package::Cpu(p)) => t.size == p.image.size,
        (Native::Yuv(t), Package::Yuv { frame, .. }) => t.compatible(frame),
        (Native::Glow(t), Package::Cpu(p)) => t.size == p.image.size,
        #[cfg(test)]
        (Native::Controlled(_, size), Package::Cpu(p)) => *size == p.image.size,
        _ => false,
    }
}
pub(super) fn set_resident(
    s: &mut SlotState,
    native: Native,
    package: Package,
    drain: Drain,
    claims: GpuClaims,
) {
    s.admission = None;
    s.resident = Some(Resident {
        native,
        upload: UploadTicket {
            drain,
            package: package.clone(),
            staging: claims.staging,
        },
        package,
        surface: claims.surface,
    });
}
#[cfg(test)]
#[path = "batches_renderer_task_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::ControlledDriver;
