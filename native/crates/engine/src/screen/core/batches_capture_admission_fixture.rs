//! Shared admission metadata and actual resource-retirement fixture owners.
use crate::screen::core::{
    capture_admission::{
        CaptureAuthority, CaptureGeneration, CaptureOwner, CapturePlan, SourceKey,
    },
    capture_grant::{CaptureGrant, CaptureSlice, DelegatedCaptureCredit},
    capture_plan::CapturePhases,
    frame_budget::{FrameContract, FrameLimits, FRAME_PROTOCOL},
    frame_memory::MemoryFacts,
    pixel_convert::FrameSize,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) const MIB: u64 = 1024 * 1024;

pub(super) fn facts() -> Option<MemoryFacts> {
    Some(MemoryFacts {
        available_physical: 4096 * MIB,
        available_commit: 4096 * MIB,
        available_address_space: 4096 * MIB,
    })
}
pub(super) fn contract(bytes: u64) -> FrameContract {
    FrameContract {
        version: FRAME_PROTOCOL,
        limits: FrameLimits {
            max_size: FrameSize::new(64, 64),
            max_pixels: 4096,
            working_bytes: bytes,
        },
        hybrid_regions: true,
    }
}
pub(super) fn plan(session: &str, source: &str, generation: u64, bytes: u64) -> CapturePlan {
    let mut plan = CapturePlan::new(
        SourceKey::new(session, source).unwrap(),
        CaptureGeneration::new(generation),
    );
    plan.add_bytes(CaptureOwner::SourcePixels, bytes, 1, CapturePhases::ALL)
        .unwrap();
    plan
}
pub(super) struct Retirement {
    pub(super) hold: Option<CaptureSlice>,
    pub(super) retired: Arc<AtomicBool>,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        drop(self.hold.take());
        self.retired.store(true, Ordering::SeqCst);
    }
}
pub(super) struct NativeResource {
    pub(super) grant: CaptureGrant,
    pub(super) observed: Arc<AtomicBool>,
}
impl Drop for NativeResource {
    fn drop(&mut self) {
        self.observed
            .store(self.grant.live_bytes().unwrap() == 4, Ordering::SeqCst);
    }
}
pub(super) struct VerifiedDispatcher;
impl CaptureAuthority for VerifiedDispatcher {
    fn approve(&self, plan: CapturePlan) -> std::io::Result<CaptureGrant> {
        let credit = DelegatedCaptureCredit::from_verified_parent(
            plan.source().clone(),
            plan.generation(),
            plan.peak_bytes(),
            plan.owner_limits(),
            (),
        )?;
        CaptureGrant::from_delegated(plan, credit)
    }
}
pub(super) struct SessionRetirement(pub(super) Arc<AtomicBool>);
impl Drop for SessionRetirement {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
