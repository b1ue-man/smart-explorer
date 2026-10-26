//! Bounded product ownership. Completion is an explicit backend event.
use super::screen_image_memory::{CpuPixels, GpuClaims};
use super::screen_presented::PresentedFrame;
use super::screen_render_owner::{lock, notify_all, Native, Resident, Retirement};
use crate::screen::core::frame_memory::{MemoryHold, MemoryLease};
use crate::screen::core::image_admission::ImageHold;
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};

#[derive(Clone)]
pub(crate) struct Generation(Arc<GenerationState>);
struct GenerationState {
    active: AtomicBool,
    guard: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    revoked: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl std::fmt::Debug for Generation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Generation")
            .field("active", &self.0.active.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}
impl PartialEq for Generation {
    fn eq(&self, rhs: &Self) -> bool {
        self.same(rhs)
    }
}
impl Eq for Generation {}
impl Default for Generation {
    fn default() -> Self {
        Self(Arc::new(GenerationState {
            active: AtomicBool::new(true),
            guard: None,
            revoked: None,
        }))
    }
}
impl Generation {
    pub(crate) fn guarded(guard: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self(Arc::new(GenerationState {
            active: AtomicBool::new(true),
            guard: Some(guard),
            revoked: None,
        }))
    }
    pub(crate) fn revocable(
        guard: Arc<dyn Fn() -> bool + Send + Sync>,
        revoked: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self(Arc::new(GenerationState {
            active: AtomicBool::new(true),
            guard: Some(guard),
            revoked: Some(revoked),
        }))
    }
    pub(crate) fn active(&self) -> bool {
        self.0.active.load(Ordering::Acquire) && self.0.guard.as_ref().is_none_or(|guard| guard())
    }
    pub(crate) fn revoke(&self) {
        if self.0.active.swap(false, Ordering::AcqRel) {
            if let Some(revoked) = &self.0.revoked {
                revoked();
            }
        }
    }
    pub(crate) fn same(&self, rhs: &Self) -> bool {
        Arc::ptr_eq(&self.0, &rhs.0)
    }
}
#[derive(Clone)]
pub(crate) struct PreparedImage {
    pub image: Arc<CpuPixels>,
    pub memory: MemoryHold,
    pub frame: Option<PresentedFrame>,
    pub generation: Generation,
    pub seq: u64,
}
#[derive(Clone, Default)]
pub(crate) struct Drain(Arc<AtomicBool>);
impl Drain {
    pub(crate) fn confirmed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub(crate) fn confirm(&self) {
        self.0.store(true, Ordering::Release);
        super::screen_render_owner::notify_all();
    }
}
pub(crate) struct UploadTicket {
    pub drain: Drain,
    pub package: Package,
    pub staging: Option<ImageHold>,
}
#[derive(Clone)]
pub(crate) enum Package {
    Cpu(PreparedImage),
    Yuv {
        frame: Arc<super::screen_wgpu_yuv::YuvFrame>,
        stamp: PresentedFrame,
        generation: Generation,
        rect: eframe::egui::Rect,
        screen: [u32; 2],
        ppp: f32,
    },
}
impl Package {
    pub(crate) fn active(&self) -> bool {
        self.generation().active() && self.frame().is_none_or(|frame| frame.active())
    }
    pub(crate) fn generation(&self) -> &Generation {
        match self {
            Self::Cpu(p) => &p.generation,
            Self::Yuv { generation, .. } => generation,
        }
    }
    pub(crate) fn frame(&self) -> Option<PresentedFrame> {
        match self {
            Self::Cpu(p) => p.frame.clone(),
            Self::Yuv { stamp, .. } => Some(stamp.clone()),
        }
    }
    pub(crate) fn memory(&self) -> MemoryHold {
        match self {
            Self::Cpu(p) => p.memory.clone(),
            Self::Yuv { frame, .. } => frame.memory.clone(),
        }
    }
    pub(super) fn revoke_presentation(&self) {
        if let Some(frame) = self.frame() {
            frame.validity.revoke();
        }
    }
    pub(super) fn upload_claims(&self, surface: Option<ImageHold>) -> std::io::Result<GpuClaims> {
        match self {
            Self::Cpu(image) => image.image.upload_claims(surface),
            Self::Yuv { .. } => Ok(GpuClaims::default()),
        }
    }
    pub(crate) fn same_image(&self, rhs: &Self) -> bool {
        match (self, rhs) {
            (Self::Cpu(a), Self::Cpu(b)) => {
                a.seq == b.seq
                    && a.image.size == b.image.size
                    && a.frame == b.frame
                    && a.generation.same(&b.generation)
            }
            (
                Self::Yuv {
                    frame: a,
                    stamp: sa,
                    generation: ga,
                    ..
                },
                Self::Yuv {
                    frame: b,
                    stamp: sb,
                    generation: gb,
                    ..
                },
            ) => Arc::ptr_eq(a, b) && sa == sb && ga.same(gb),
            _ => false,
        }
    }
}

pub(crate) struct RenderSlot {
    pub(super) state: Mutex<SlotState>,
}
pub(super) struct SlotState {
    pub(super) generation: Generation,
    pub(super) closed: bool,
    pub(super) next_upload: Option<GpuClaims>,
    pub candidate: Option<Package>,
    pub resident: Option<Resident>,
    pub(super) retiring: Option<Retirement>,
    pub(super) admission: Option<Weak<MemoryLease>>,
}
impl SlotState {
    pub(super) fn revoke_presentations(&self) {
        for package in self
            .candidate
            .iter()
            .chain(self.resident.iter().map(|resident| &resident.package))
        {
            package.revoke_presentation();
        }
        if let Some(retiring) = &self.retiring {
            retiring.package.revoke_presentation();
            retiring.upload.package.revoke_presentation();
            if let Some(previous) = &retiring.superseded {
                previous.revoke_presentation();
            }
        }
    }
    pub(super) fn quarantine(&mut self) {
        self.revoke_presentations();
        self.generation.revoke();
        self.candidate = None;
        self.next_upload = None;
    }
    pub(super) fn claim_upload(&mut self, package: &Package) -> std::io::Result<GpuClaims> {
        match self.next_upload.take() {
            Some(claims) => Ok(claims),
            None => package.upload_claims(self.resident.as_ref().and_then(|r| r.surface.clone())),
        }
    }
}
impl Default for RenderSlot {
    fn default() -> Self {
        Self {
            state: Mutex::new(SlotState {
                generation: Generation::default(),
                closed: false,
                next_upload: None,
                candidate: None,
                resident: None,
                retiring: None,
                admission: None,
            }),
        }
    }
}
impl RenderSlot {
    pub(crate) fn generation(&self) -> Generation {
        lock(&self.state).generation.clone()
    }
    pub(crate) fn accepting(&self) -> bool {
        let s = lock(&self.state);
        !s.closed && s.next_upload.is_none()
    }
    pub(crate) fn queue(&self, package: Package) -> bool {
        let mut s = lock(&self.state);
        // A resize already owns its exact future surface/upload. Keep that
        // candidate stable until handoff; replacing it would lose its admission.
        if !s.closed
            && s.next_upload.is_none()
            && package.generation().same(&s.generation)
            && package.active()
        {
            s.candidate = Some(package);
            true
        } else {
            false
        }
    }
    pub(crate) fn watch_memory(&self, memory: &MemoryHold) {
        if let Some(memory) = memory {
            lock(&self.state).admission = Some(Arc::downgrade(memory));
        }
    }
    pub(crate) fn reset(&self) {
        let mut s = lock(&self.state);
        s.generation.revoke();
        s.generation = Generation::default();
        s.candidate = None;
        s.next_upload = None;
        if let Some(r) = &s.resident {
            s.admission = r.package.memory().as_ref().map(Arc::downgrade);
        }
        drop(s);
        notify_all();
    }
    pub(crate) fn close(&self) {
        self.reset();
        lock(&self.state).closed = true;
        notify_all();
    }
    pub(crate) fn frame(&self) -> Option<super::screen_presented::PresentedFrame> {
        lock(&self.state)
            .resident
            .as_ref()
            .filter(|r| r.package.active())
            .and_then(|r| r.package.frame())
    }
    pub(crate) fn id(&self) -> Option<egui::TextureId> {
        lock(&self.state)
            .resident
            .as_ref()
            .filter(|r| r.package.active())
            .and_then(|r| match r.native {
                Native::Wgpu(ref t) => Some(t.id),
                Native::Glow(ref t) => Some(t.id),
                #[cfg(test)]
                Native::Controlled(id, _) => Some(id),
                _ => None,
            })
    }
}
